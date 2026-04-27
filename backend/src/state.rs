use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::Mutex as PlMutex;
use tokio::sync::Mutex as TokioMutex;
use serde::Serialize;
use crate::db::Db;
use crate::relay::{RelayController, RelayState};

/// Capacité du buffer live (5 minutes à 1 Hz). Le buffer vit dans `InnerState` pour
/// que les sparklines soient préremplies dès le premier rendu, sans devoir attendre
/// 5 min après chaque rechargement de page.
pub const LIVE_BUFFER_CAPACITY: usize = 300;

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
    /// Buffer circulaire 1 Hz × 5 min des dernières mesures. Mis à jour par
    /// la loop sensors qui pousse à chaque tick (1 s) une ligne alignée
    /// contenant les tensions des deux capteurs et les tensions UPS courantes.
    pub live: LiveBuffer,
}

/// Buffer circulaire des dernières secondes. Toutes les `VecDeque` ont la
/// même longueur (alignées par index sur `ts`).
pub struct LiveBuffer {
    pub ts: VecDeque<i64>,
    pub sensor_grid_v: VecDeque<Option<f32>>,    // 0x40
    pub sensor_solar_v: VecDeque<Option<f32>>,   // 0x41
    pub ups_input_v: VecDeque<Option<f32>>,
    pub ups_battery_v: VecDeque<Option<f32>>,
}

impl LiveBuffer {
    pub fn new() -> Self {
        Self {
            ts: VecDeque::with_capacity(LIVE_BUFFER_CAPACITY),
            sensor_grid_v: VecDeque::with_capacity(LIVE_BUFFER_CAPACITY),
            sensor_solar_v: VecDeque::with_capacity(LIVE_BUFFER_CAPACITY),
            ups_input_v: VecDeque::with_capacity(LIVE_BUFFER_CAPACITY),
            ups_battery_v: VecDeque::with_capacity(LIVE_BUFFER_CAPACITY),
        }
    }

    pub fn push(
        &mut self,
        ts: i64,
        sensors: &[SensorReading],
        ups: Option<&UpsReading>,
    ) {
        if self.ts.len() >= LIVE_BUFFER_CAPACITY {
            self.ts.pop_front();
            self.sensor_grid_v.pop_front();
            self.sensor_solar_v.pop_front();
            self.ups_input_v.pop_front();
            self.ups_battery_v.pop_front();
        }
        self.ts.push_back(ts);
        self.sensor_grid_v.push_back(value_for(sensors, 0x40));
        self.sensor_solar_v.push_back(value_for(sensors, 0x41));
        self.ups_input_v.push_back(ups.and_then(|u| u.input_voltage_v));
        self.ups_battery_v.push_back(ups.and_then(|u| u.battery_voltage_v));
    }
}

fn value_for(sensors: &[SensorReading], addr: u8) -> Option<f32> {
    sensors.iter().find(|s| s.address == addr).map(|s| s.bus_voltage_v)
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

    /// Pool PostgreSQL + flag de connectivité. `None` si la DB n'est pas
    /// configurée ou injoignable au boot — le service tourne alors en mode
    /// dégradé (pas de recorder, pas d'history). Toujours `None` au moment de
    /// `AppState::new` ; assigné dans `main` APRÈS le switch initial vers Grid.
    pub db: Option<Db>,
}

impl AppState {
    pub fn new(relay: RelayController) -> Self {
        let initial_state = relay.current_state();
        AppState {
            inner: Arc::new(PlMutex::new(InnerState {
                sensors: vec![],
                ups: None,
                published_state: initial_state,
                live: LiveBuffer::new(),
            })),
            relay: Arc::new(TokioMutex::new(relay)),
            db: None,
        }
    }
}
