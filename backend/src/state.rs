use std::sync::Arc;
use parking_lot::Mutex as PlMutex;
use tokio::sync::Mutex as TokioMutex;
use serde::Serialize;
use crate::relay::{RelayController, RelayState};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Grid,
    Solar,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorReading {
    pub address: u8,
    pub bus_voltage_v: f32,
    pub current_ma: f32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UpsReading {
    pub input_voltage_v: Option<f32>,
    pub input_frequency_hz: Option<f32>,
    pub output_voltage_v: Option<f32>,
    pub load_pct: Option<f32>,
    pub battery_pct: Option<f32>,
    pub battery_voltage_v: Option<f32>,
    pub runtime_s: Option<u32>,
    pub status: Option<String>,
    pub last_seen: i64,
}

pub struct InnerState {
    pub sensors: Vec<SensorReading>,
    pub ups: Option<UpsReading>,
    /// État du contrôleur publié pour `/api/status`. Mis à jour par
    /// `post_switch` après un switch_to OK et par le watchdog en cas de
    /// correction. Permet de répondre au status sans bloquer sur le mutex
    /// du relay (qui est tenu pendant toute la durée d'un switch).
    pub published_state: RelayState,
}

#[derive(Clone)]
pub struct AppState {
    /// Données légères mutées en synchrone (capteurs).
    /// `parking_lot::Mutex` ne s'empoisonne pas en cas de panic, donc pas de
    /// cascade de paniques.
    pub inner: Arc<PlMutex<InnerState>>,

    /// Contrôleur des relais. Encapsule TOUS les invariants de sécurité.
    /// `tokio::sync::Mutex` permet de tenir le verrou pendant les `await`
    /// internes à `switch_to`. L'exclusion mutuelle empêche par construction
    /// deux switchs concurrents.
    pub relay: Arc<TokioMutex<RelayController>>,
}

impl AppState {
    pub fn new(relay: RelayController) -> Self {
        let initial_state = relay.current_state();
        AppState {
            inner: Arc::new(PlMutex::new(InnerState {
                sensors: vec![],
                ups: None,
                published_state: initial_state,
            })),
            relay: Arc::new(TokioMutex::new(relay)),
        }
    }
}
