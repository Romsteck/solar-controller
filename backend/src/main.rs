mod auto;
mod db;
mod history;
mod recorder;
mod relay;
mod routes;
mod sensors;
mod state;
mod ups;
mod watchdog;
mod weather;

use std::time::Duration;
use axum::{
    http::{header, HeaderValue},
    routing::{get, post},
    Router,
};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use crate::state::Network;

#[cfg(unix)]
async fn wait_for_shutdown_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    match (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) {
        (Ok(mut sigterm), Ok(mut sigint)) => {
            tokio::select! {
                _ = sigterm.recv() => tracing::info!("SIGTERM reçu"),
                _ = sigint.recv() => tracing::info!("SIGINT reçu"),
            }
        }
        _ => {
            // Fallback : on attend Ctrl-C uniquement.
            let _ = tokio::signal::ctrl_c().await;
            tracing::info!("Ctrl-C reçu");
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("Ctrl-C reçu");
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    // Panic hook : log avant que le process ne meure. Les OutputPin sont
    // configurés `set_reset_on_drop(true)` ; et même sans Drop, le kernel
    // libère les lignes GPIO à la sortie du process → mode input → HAT
    // pull-ups → relais ouverts.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("!!! PANIC : {info}");
        default_hook(info);
    }));

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 1 : init GPIO. Les pins partent en HIGH = relais ouverts (sûr).
    // ═══════════════════════════════════════════════════════════════════════
    let relay = match relay::RelayController::new() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "Init GPIO échouée");
            std::process::exit(1);
        }
    };
    let mut app_state = state::AppState::new(relay);

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 2 (LOAD-BEARING) : basculer en Grid AVANT toute init réseau.
    // Ne JAMAIS introduire un await réseau (DB connect, HTTP fetch, DNS) avant
    // ce bloc — sinon un timeout réseau au boot laisserait l'UPS sur batterie.
    // Si le switch initial échoue, on reste relais ouverts (état sûr).
    // ═══════════════════════════════════════════════════════════════════════
    {
        let mut relay = app_state.relay.lock().await;
        match relay.switch_to(Network::Grid, Duration::from_millis(500)).await {
            Ok(()) => tracing::info!("Boot OK : mode Grid"),
            Err(e) => tracing::error!(
                error = %e,
                "Switch initial vers Grid échoué — relais restent ouverts"
            ),
        }
        let new_state = relay.current_state();
        app_state.inner.lock().published_state = new_state;
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 3 : init DB en mode "lazy". Le pool est construit sans contacter
    // PostgreSQL — la connexion réelle se fait à la première query, et
    // `db::health_loop` retente toutes les 10s en cas d'échec persistant.
    // Si la DB est injoignable au boot, le service démarre quand même : les
    // loops dépendantes (recorder, weather) écriront en best-effort, et le
    // mode "connected" se remettra automatiquement quand la DB reviendra.
    // ═══════════════════════════════════════════════════════════════════════
    let db = std::env::var("DATABASE_URL")
        .ok()
        .and_then(|url| match db::connect_lazy(&url) {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::error!(error = %e, "URL DB invalide — mode dégradé permanent");
                None
            }
        });
    if db.is_none() {
        tracing::warn!("DATABASE_URL non définie/invalide — mode dégradé (pas de persistance)");
    }
    app_state.db = db.clone();

    // Tentative initiale de connexion (best-effort, 5s max). Permet de
    // charger `auto_enabled` rapidement si la DB est dispo. Sinon fallback
    // à `true` et `health_loop` reprendra dans 10s.
    let auto_enabled = match db.as_ref() {
        Some(d) => {
            let connected = matches!(
                tokio::time::timeout(Duration::from_secs(5), db::try_connect_and_init(d)).await,
                Ok(true)
            );
            if connected {
                db::get_setting_bool(d, "auto_enabled", true).await
            } else {
                tracing::warn!(
                    "DB indispo au boot — auto_enabled défaut à true, retry health_loop /10s"
                );
                true
            }
        }
        None => true,
    };
    app_state.inner.lock().auto.enabled = auto_enabled;
    tracing::info!(auto_enabled, "Settings chargés");

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 4 : spawn des loops. Sensors/UPS/watchdog/auto tournent toujours.
    // Recorder/weather/health-loop tournent dès que DATABASE_URL est définie,
    // peu importe l'état actuel de la DB — chacune se rabat en best-effort
    // si is_connected()=false, et reprend automatiquement à la reconnexion.
    // ═══════════════════════════════════════════════════════════════════════
    tokio::spawn(sensors::poll_loop(app_state.clone()));
    tokio::spawn(ups::poll_loop(app_state.clone()));
    tokio::spawn(watchdog::run(app_state.clone()));
    // La boucle auto tourne même sans DB : la règle 1 (urgence tension) est notre
    // filet de sécurité ultime, indépendante de la persistance.
    tokio::spawn(auto::run(app_state.clone()));

    if let Some(d) = db.clone() {
        tokio::spawn(db::health_loop(d.clone()));
        tokio::spawn(recorder::record_loop(app_state.clone(), d.clone()));

        let lat = std::env::var("WEATHER_LAT")
            .ok()
            .and_then(|v| v.parse::<f32>().ok());
        let lon = std::env::var("WEATHER_LON")
            .ok()
            .and_then(|v| v.parse::<f32>().ok());
        match (lat, lon) {
            (Some(la), Some(lo)) => {
                tokio::spawn(weather::weather_loop(d, la, lo));
            }
            _ => tracing::warn!("WEATHER_LAT/WEATHER_LON manquants — météo désactivée"),
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 5 : signal handler pour graceful shutdown.
    // ═══════════════════════════════════════════════════════════════════════
    let shutdown_state = app_state.clone();
    tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        tracing::info!("Arrêt demandé : ouverture des relais avant exit");
        let timeout = Duration::from_secs(3);
        match tokio::time::timeout(timeout, shutdown_state.relay.lock()).await {
            Ok(mut relay) => {
                relay.open_all();
                drop(relay);
            }
            Err(_) => {
                // Si on n'arrive pas à acquérir le lock, le kernel libère les
                // lignes GPIO à la sortie → relais ouverts via les pull-ups.
                tracing::error!("Timeout acquisition lock relay au shutdown");
            }
        }
        std::process::exit(0);
    });

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 6 : serve HTTP.
    // Compression gzip uniquement sur les routes API (pas sur les assets
    // statiques pré-compressés par Vite).
    // ═══════════════════════════════════════════════════════════════════════
    // Layers appliqués UNIQUEMENT aux routes /api/* (pas au fallback statique).
    // - Connection: close → désactive le keep-alive HTTP sur les endpoints
    //   polés en boucle (status à 1 Hz). Évite le pile-up de sockets en
    //   CLOSE-WAIT que des clients externes (Grafana/HA, etc.) provoquent
    //   en ne fermant pas proprement leurs connexions keep-alive.
    // - TimeoutLayer 15s → tue toute requête qui traîne (handler bloqué,
    //   DB lente, etc.) plutôt que d'occuper un fd indéfiniment.
    // Les assets statiques (frontend/dist) gardent keep-alive (chargés
    //   rarement, bénéficient du multiplexage navigateur).
    let app = Router::new()
        .route("/api/status", get(routes::get_status))
        .route("/api/switch", post(routes::post_switch))
        .route("/api/auto", post(routes::post_auto))
        .route("/api/history", get(routes::get_history))
        .route("/api/live-history", get(routes::get_live_history))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONNECTION,
            HeaderValue::from_static("close"),
        ))
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(15),
        ))
        .layer(CompressionLayer::new())
        .fallback_service(ServeDir::new("frontend/dist"))
        .with_state(app_state);

    let listener = match tokio::net::TcpListener::bind("0.0.0.0:3000").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, "Bind 0.0.0.0:3000 échoué");
            std::process::exit(1);
        }
    };
    tracing::info!("Listening on http://0.0.0.0:3000");
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(error = %e, "Erreur serveur HTTP");
        std::process::exit(1);
    }
}
