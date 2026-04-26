mod relay;
mod routes;
mod sensors;
mod state;
mod ups;
mod watchdog;

use std::time::Duration;
use axum::{routing::{get, post}, Router};
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

    let relay = match relay::RelayController::new() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "Init GPIO échouée");
            std::process::exit(1);
        }
    };
    let app_state = state::AppState::new(relay);

    // Boot en mode nominal : après l'init où les deux pins sont HIGH (relais
    // ouverts), on entre en mode Grid via la même API sécurisée que le runtime.
    // Si l'opération échoue, on reste en RelayState::Open — état sûr.
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

    tokio::spawn(sensors::poll_loop(app_state.clone()));
    tokio::spawn(ups::poll_loop(app_state.clone()));
    tokio::spawn(watchdog::run(app_state.clone()));

    // Handler de signal pour un graceful shutdown. À la réception de
    // SIGTERM/SIGINT, on force open_all avant de quitter.
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

    let app = Router::new()
        .route("/api/status", get(routes::get_status))
        .route("/api/switch", post(routes::post_switch))
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
