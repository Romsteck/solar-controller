use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use std::time::Duration;
use crate::state::AppState;
use crate::relay::RelayState;

/// Délai « UX » que `post_switch` demande à `switch_to`. Le `RelayController`
/// applique en interne `max(SWITCH_UX_DELAY, RELAY_SETTLE_MIN)` ; donc même si
/// quelqu'un règle ici une valeur trop faible, le délai mécanique de sécurité
/// est garanti.
const SWITCH_UX_DELAY: Duration = Duration::from_secs(2);

#[derive(Serialize)]
pub struct StatusResponse {
    relay_state: RelayState,
    switching: bool,
    sensors: Vec<crate::state::SensorReading>,
    ups: Option<crate::state::UpsReading>,
}

pub async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
    // `try_lock` sur le mutex du relay est notre indicateur de transition :
    // si le mutex est tenu par `switch_to`, c'est qu'un switch est en cours.
    let switching = state.relay.try_lock().is_err();
    let inner = state.inner.lock();
    Json(StatusResponse {
        relay_state: inner.published_state,
        switching,
        sensors: inner.sensors.clone(),
        ups: inner.ups.clone(),
    })
}

pub async fn post_switch(State(state): State<AppState>) -> impl IntoResponse {
    // `try_lock` : si un switch est déjà en cours, retourner 409 immédiatement
    // au lieu de mettre la requête en file (sinon des clics multiples
    // empileraient des switchs successifs alors que l'utilisateur n'en a
    // demandé qu'un seul).
    let mut relay = match state.relay.try_lock() {
        Ok(g) => g,
        Err(_) => {
            return (StatusCode::CONFLICT, "switch déjà en cours").into_response();
        }
    };

    let target = relay.current_state().next_target();
    tracing::info!(?target, current = ?relay.current_state(), "début switch");

    let result = relay.switch_to(target, SWITCH_UX_DELAY).await;
    let new_state = relay.current_state();
    // Publier l'état (nouveau si succès, Open si erreur car switch_to force open_all sur erreur).
    state.inner.lock().published_state = new_state;

    match result {
        Ok(()) => {
            tracing::info!(?new_state, "switch OK");
            StatusCode::ACCEPTED.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "switch ÉCHOUÉ — relais forcés ouverts");
            (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response()
        }
    }
}
