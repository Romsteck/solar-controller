use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;
use crate::db::{log_relay_event, set_setting_bool};
use crate::history::{fetch_history, Range};
use crate::relay::RelayState;
use crate::state::AppState;

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
    db_connected: bool,
    auto_enabled: bool,
    auto_reason: Option<String>,
    auto_message: Option<String>,
    soc_percent: Option<f32>,
    float_reached_today: bool,
    eod_lockout: bool,
    eod_at: Option<chrono::DateTime<Utc>>,
    eod_threshold_v: Option<f32>,
}

/// Buffer live 5 min × 1 Hz, sérialisé orienté série pour minimiser les bytes.
#[derive(Serialize)]
pub struct LiveHistoryResponse {
    capacity: usize,
    ts: Vec<i64>,
    sensor_grid_v: Vec<Option<f32>>,
    sensor_solar_v: Vec<Option<f32>>,
    ups_input_v: Vec<Option<f32>>,
    ups_battery_v: Vec<Option<f32>>,
}

pub async fn get_live_history(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.inner.lock();
    Json(LiveHistoryResponse {
        capacity: crate::state::LIVE_BUFFER_CAPACITY,
        ts: inner.live.ts.iter().copied().collect(),
        sensor_grid_v: inner.live.sensor_grid_v.iter().copied().collect(),
        sensor_solar_v: inner.live.sensor_solar_v.iter().copied().collect(),
        ups_input_v: inner.live.ups_input_v.iter().copied().collect(),
        ups_battery_v: inner.live.ups_battery_v.iter().copied().collect(),
    })
}

pub async fn get_status(State(state): State<AppState>) -> impl IntoResponse {
    // `try_lock` sur le mutex du relay est notre indicateur de transition :
    // si le mutex est tenu par `switch_to`, c'est qu'un switch est en cours.
    let switching = state.relay.try_lock().is_err();
    let db_connected = state.db.as_ref().map(|d| d.is_connected()).unwrap_or(false);
    let inner = state.inner.lock();
    let (auto_reason, auto_message) = match &inner.auto.last_decision {
        Some(d) => (Some(d.reason.clone()), Some(d.message.clone())),
        None => (None, None),
    };
    Json(StatusResponse {
        relay_state: inner.published_state,
        switching,
        sensors: inner.sensors.clone(),
        ups: inner.ups.clone(),
        db_connected,
        auto_enabled: inner.auto.enabled,
        auto_reason,
        auto_message,
        soc_percent: inner.auto.soc_percent,
        float_reached_today: inner.auto.float_reached_today,
        eod_lockout: inner.auto.eod_lockout,
        eod_at: inner.auto.eod_at,
        eod_threshold_v: inner.auto.eod_threshold_v,
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
    let now = Utc::now();
    // Publier l'état (nouveau si succès, Open si erreur car switch_to force open_all sur erreur).
    // `last_switch_at` arme l'anti-oscillation 10 min — c'est ce qui empêche
    // l'auto de défaire le switch utilisateur immédiatement.
    {
        let mut inner = state.inner.lock();
        inner.published_state = new_state;
        inner.auto.last_switch_at = Some(now);
    }

    // Audit trail DB (best-effort).
    if let Some(db) = state.db.as_ref() {
        if db.is_connected() {
            let state_str = match new_state {
                RelayState::Open => "open",
                RelayState::Grid => "grid",
                RelayState::Solar => "solar",
            };
            let reason_str = if result.is_ok() {
                "manual"
            } else {
                "manual_failed"
            };
            if let Err(e) = log_relay_event(db, now, state_str, reason_str).await {
                tracing::warn!(error = %e, "log_relay_event échoué");
            }
        }
    }

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

#[derive(Deserialize)]
pub struct AutoToggleBody {
    enabled: bool,
}

#[derive(Serialize)]
pub struct AutoToggleResponse {
    auto_enabled: bool,
}

pub async fn post_auto(
    State(state): State<AppState>,
    Json(body): Json<AutoToggleBody>,
) -> impl IntoResponse {
    let db = match state.db.as_ref() {
        Some(d) if d.is_connected() => d,
        _ => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "DB indisponible — toggle non persisté",
            )
                .into_response();
        }
    };

    if let Err(e) = set_setting_bool(db, "auto_enabled", body.enabled).await {
        tracing::error!(error = %e, "set_setting_bool auto_enabled échoué");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Erreur DB: {e}"),
        )
            .into_response();
    }

    {
        let mut inner = state.inner.lock();
        inner.auto.enabled = body.enabled;
        // Reset des compteurs : on évite qu'un toggle hâtif déclenche un switch
        // au tick suivant sur la base d'une accumulation passée.
        inner.auto.low_voltage_minutes = 0;
        inner.auto.recover_voltage_minutes = 0;
    }

    tracing::info!(enabled = body.enabled, "Auto toggle persisté");
    Json(AutoToggleResponse {
        auto_enabled: body.enabled,
    })
    .into_response()
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    range: Option<String>,
}

pub async fn get_history(
    State(state): State<AppState>,
    Query(params): Query<HistoryQuery>,
) -> impl IntoResponse {
    let db = match state.db.as_ref() {
        Some(d) if d.is_connected() => d,
        Some(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "DB déconnectée — réessayer plus tard",
            )
                .into_response()
        }
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "DB non configurée",
            )
                .into_response()
        }
    };

    let range = params
        .range
        .as_deref()
        .and_then(|s| Range::from_str(s).ok())
        .unwrap_or(Range::Hour);

    match fetch_history(db, range).await {
        Ok(payload) => Json(payload).into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "fetch_history a échoué");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Erreur DB: {e}"),
            )
                .into_response()
        }
    }
}
