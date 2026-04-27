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
use axum::{routing::{get, post}, Router};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
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
    // ÉTAPE 3 : init DB (avec retry borné, ~21s max). Le relais est déjà
    // sécurisé en Grid, donc un timeout DB ici n'a pas d'impact sur la charge.
    // Si la DB est injoignable, on passe en mode dégradé (pas de recorder, pas
    // de météo, /api/history → 503). Le live /api/status fonctionne toujours.
    // ═══════════════════════════════════════════════════════════════════════
    let database_url = std::env::var("DATABASE_URL").ok();
    let db = match database_url {
        Some(url) => db::connect_with_retry(&url).await,
        None => {
            tracing::warn!("DATABASE_URL non définie — mode dégradé (pas de persistance)");
            None
        }
    };
    app_state.db = db.clone();

    // Charger les settings persistés (auto_enabled). Défaut `true` si DB
    // injoignable ou clé absente — l'utilisateur a demandé auto-ON par défaut.
    let auto_enabled = match db.as_ref() {
        Some(d) => db::get_setting_bool(d, "auto_enabled", true).await,
        None => true,
    };
    app_state.inner.lock().auto.enabled = auto_enabled;
    tracing::info!(auto_enabled, "Settings chargés");

    // ═══════════════════════════════════════════════════════════════════════
    // ÉTAPE 4 : spawn des loops. Sensors/UPS/watchdog/auto tournent toujours.
    // Recorder/weather/health-check uniquement si DB OK.
    // ═══════════════════════════════════════════════════════════════════════
    tokio::spawn(sensors::poll_loop(app_state.clone()));
    tokio::spawn(ups::poll_loop(app_state.clone()));
    tokio::spawn(watchdog::run(app_state.clone()));
    // La boucle auto tourne même sans DB : la règle 1 (urgence tension) est notre
    // filet de sécurité ultime, indépendante de la persistance.
    tokio::spawn(auto::run(app_state.clone()));

    if let Some(d) = db.clone() {
        tokio::spawn(recorder::record_loop(app_state.clone(), d));
    }
    if let Some(d) = db.clone() {
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
    if let Some(d) = db.clone() {
        tokio::spawn(db::health_loop(d));
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
    let app = Router::new()
        .route("/api/status", get(routes::get_status))
        .route("/api/switch", post(routes::post_switch))
        .route("/api/auto", post(routes::post_auto))
        .route("/api/history", get(routes::get_history))
        .route("/api/live-history", get(routes::get_live_history))
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
