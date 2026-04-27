//! Boucle de décision auto-switch (Grid ⇄ Solar) basée tension + heure + météo.
//!
//! Une décision est prise toutes les 60 secondes. La règle 1 (urgence
//! V_inst < V_EMERGENCY) tourne même en mode dégradé (DB injoignable, auto
//! désactivé) — c'est le filet de sécurité ultime côté tension.
//!
//! La logique est documentée dans `je-veux-auto-par-wobbly-star.md`.

use std::time::Duration;
use chrono::{DateTime, Utc};
use crate::db::{fetch_forecast_window, log_relay_event, ForecastDay};
use crate::relay::RelayState;
use crate::state::{AppState, AutoDecision, AutoState, Network};

/// Settle UX que la boucle auto demande à `switch_to`. Le `RelayController`
/// applique en interne `max(SWITCH_AUTO_DELAY, RELAY_SETTLE_MIN)`.
const SWITCH_AUTO_DELAY: Duration = Duration::from_secs(2);

// ─────────────────────────────────────────────────────────────────────────
// Constantes — calibrées pour batterie 24V plomb (2× Hankook DC31MF en série)
// avec MPPT Epever Tracer 4210AN (Float 27.2V, Boost 28.4V).
// ─────────────────────────────────────────────────────────────────────────

/// Tension batterie instantanée sous laquelle on bascule GRID immédiatement.
/// Filet de sécurité ultime (≈50% SoC sous charge).
const V_EMERGENCY: f32 = 24.8;

/// Tension batterie sous laquelle on bascule GRID si tenu plusieurs minutes.
const V_LOW: f32 = 25.2;
const V_LOW_MIN_MINUTES: u32 = 3;

/// Tension batterie au-dessus de laquelle on autorise SOLAR si tenu plusieurs
/// minutes (hystérésis vs `V_LOW`).
const V_RECOVER: f32 = 26.2;
const V_RECOVER_MIN_MINUTES: u32 = 5;

/// Tension batterie au-delà de laquelle on considère que le MPPT est en Float
/// (charge complète atteinte). 27.2V = Float Tracer 4210AN.
const V_FLOAT: f32 = 27.2;
const V_FLOAT_MIN_MINUTES: u32 = 10;

/// Délai entre `now` et `sunset` à partir duquel on déclenche la fenêtre EOD.
const EOD_OFFSET: chrono::Duration = chrono::Duration::hours(2);

/// Anti-oscillation : pas de switch auto si < 10 min depuis le dernier (sauf urgence).
/// C'est aussi notre seul mécanisme pour respecter un switch manuel : l'auto
/// ne pourra pas défaire la décision utilisateur pendant ces 10 min.
const MIN_SWITCH_GAP: chrono::Duration = chrono::Duration::minutes(10);

/// Période de la boucle de décision.
const TICK: Duration = Duration::from_secs(60);

// ─────────────────────────────────────────────────────────────────────────
// SoC : interpolation linéaire par segments à partir de la tension batterie.
// ─────────────────────────────────────────────────────────────────────────

const SOC_TABLE: &[(f32, f32)] = &[
    // (V, SoC%)
    (23.6, 0.0),
    (24.4, 30.0),
    (24.8, 50.0),
    (25.0, 60.0),
    (25.6, 75.0),
    (26.4, 90.0),
    (27.2, 100.0),
];

/// Estimation grossière du SoC en % à partir de la tension batterie.
/// Hors plage : clamp à 0% / 100%.
pub fn soc_from_voltage(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let first = SOC_TABLE[0];
    if v <= first.0 {
        return 0.0;
    }
    let last = SOC_TABLE[SOC_TABLE.len() - 1];
    if v >= last.0 {
        return 100.0;
    }
    for w in SOC_TABLE.windows(2) {
        let (v0, s0) = w[0];
        let (v1, s1) = w[1];
        if v >= v0 && v <= v1 {
            let t = (v - v0) / (v1 - v0);
            return s0 + t * (s1 - s0);
        }
    }
    0.0
}

// ─────────────────────────────────────────────────────────────────────────
// Décision : pure fonction sur l'état + entrées. Testable sans I/O.
// ─────────────────────────────────────────────────────────────────────────

/// Action proposée par la machine à états.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Aucune bascule, juste mise à jour des compteurs / journalisation.
    Hold,
    /// Forcer GRID.
    SwitchToGrid,
    /// Autoriser SOLAR.
    SwitchToSolar,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub action: Action,
    /// Identifiant stable (snake_case) — sert de clé dans `relay_events.reason`
    /// et pour la sérialisation API.
    pub reason: &'static str,
    /// Phrase humaine courte (FR).
    pub message: String,
}

/// Entrées de la machine à états — extraites du monde réel par `run`.
#[derive(Debug, Clone)]
pub struct DecisionInputs {
    pub now: DateTime<Utc>,
    pub current_relay: RelayState,
    pub voltage_inst: Option<f32>,
    pub voltage_max5min: Option<f32>,
    /// Today's sunrise/sunset si connus. `None` → pas de logique horaire.
    pub today_sunrise: Option<DateTime<Utc>>,
    pub today_sunset: Option<DateTime<Utc>>,
    /// Prévision rayonnement total demain (kWh/m²). `None` → fallback nominal.
    pub tomorrow_radiation_kwh: Option<f32>,
}

/// Calcule le seuil EOD selon la prévision de demain et le drapeau Float
/// atteint aujourd'hui. Cf. table dans je-veux-auto-par-wobbly-star.md.
pub fn eod_threshold(tomorrow_radiation_kwh: Option<f32>, float_reached_today: bool) -> f32 {
    let base = match tomorrow_radiation_kwh {
        Some(r) if r >= 4.0 => 26.3,
        Some(r) if r >= 2.0 => 26.7,
        Some(_) => 27.0,
        None => 26.7, // pas de data → nominal
    };
    if float_reached_today {
        base
    } else {
        base + 0.2
    }
}

/// Cœur de la décision (pure). Pas d'I/O, pas de mutation, juste calcul.
/// Lit `auto` en lecture seule ; les compteurs sont mis à jour par `run`.
pub fn decide(auto: &AutoState, inputs: &DecisionInputs) -> Decision {
    // Règle 1 : urgence (toujours active, même si auto désactivé).
    if let Some(v) = inputs.voltage_inst {
        if v < V_EMERGENCY {
            return Decision {
                action: if inputs.current_relay == RelayState::Grid {
                    Action::Hold
                } else {
                    Action::SwitchToGrid
                },
                reason: "emergency_low_voltage",
                message: format!("Urgence : tension batterie {:.2}V < {:.1}V", v, V_EMERGENCY),
            };
        }
    }

    // Si auto désactivé, on s'arrête là (seule la règle 1 ci-dessus court).
    if !auto.enabled {
        return Decision {
            action: Action::Hold,
            reason: "auto_disabled",
            message: "Auto-switch désactivé".to_string(),
        };
    }

    // Anti-oscillation : pas de switch auto si dernier switch trop récent.
    // C'est ce qui protège un switch manuel d'être défait dans la minute qui
    // suit (chaque POST /api/switch met à jour `last_switch_at`).
    let recent_switch = auto
        .last_switch_at
        .map(|t| inputs.now.signed_duration_since(t) < MIN_SWITCH_GAP)
        .unwrap_or(false);

    // Règle 2 : fenêtre EOD.
    if let Some(sunset) = inputs.today_sunset {
        let eod_start = sunset - EOD_OFFSET;
        if inputs.now >= eod_start && !auto.eod_lockout {
            let v = inputs.voltage_max5min.unwrap_or(0.0);
            let threshold = eod_threshold(
                inputs.tomorrow_radiation_kwh,
                auto.float_reached_today,
            );
            if v < threshold {
                let action = if inputs.current_relay == RelayState::Grid {
                    Action::Hold
                } else if recent_switch {
                    return Decision {
                        action: Action::Hold,
                        reason: "anti_oscillation",
                        message: "EOD souhaité mais switch trop récent".to_string(),
                    };
                } else {
                    Action::SwitchToGrid
                };
                return Decision {
                    action,
                    reason: "eod_recharge",
                    message: format!(
                        "Fin de journée : V {:.2}V < seuil {:.1}V (recharge MPPT)",
                        v, threshold
                    ),
                };
            }
        }
    }

    // Règle 3 : tension basse soutenue.
    if auto.low_voltage_minutes >= V_LOW_MIN_MINUTES
        && inputs.current_relay != RelayState::Grid
    {
        if recent_switch {
            return Decision {
                action: Action::Hold,
                reason: "anti_oscillation",
                message: "Tension basse mais switch trop récent".to_string(),
            };
        }
        return Decision {
            action: Action::SwitchToGrid,
            reason: "voltage_low_sustained",
            message: format!(
                "Tension < {:.1}V depuis {} min",
                V_LOW, auto.low_voltage_minutes
            ),
        };
    }

    // Règle 4 : reprise SOLAR autorisée — V remontée + dans la fenêtre solaire +
    // pas en lockout EOD.
    if !auto.eod_lockout
        && auto.recover_voltage_minutes >= V_RECOVER_MIN_MINUTES
        && in_solar_window(inputs)
        && inputs.current_relay != RelayState::Solar
    {
        if recent_switch {
            return Decision {
                action: Action::Hold,
                reason: "anti_oscillation",
                message: "Reprise SOLAR souhaitée mais switch trop récent".to_string(),
            };
        }
        return Decision {
            action: Action::SwitchToSolar,
            reason: "voltage_recovered",
            message: format!(
                "Tension ≥ {:.1}V depuis {} min — reprise solaire",
                V_RECOVER, auto.recover_voltage_minutes
            ),
        };
    }

    Decision {
        action: Action::Hold,
        reason: "hold",
        message: "Conditions stables".to_string(),
    }
}

fn in_solar_window(inputs: &DecisionInputs) -> bool {
    match (inputs.today_sunrise, inputs.today_sunset) {
        (Some(sr), Some(ss)) => inputs.now >= sr && inputs.now <= ss - EOD_OFFSET,
        _ => true, // pas de data soleil → on n'empêche pas la reprise
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Sélection (today / tomorrow) dans la fenêtre forecast.
// ─────────────────────────────────────────────────────────────────────────

/// Trouve la ligne "today" (celle dont la sunrise est la plus récente <= now)
/// et la ligne "tomorrow" (date+1). Retourne `(None, None)` si la fenêtre est
/// vide.
pub fn select_today_tomorrow<'a>(
    forecast: &'a [ForecastDay],
    now: DateTime<Utc>,
) -> (Option<&'a ForecastDay>, Option<&'a ForecastDay>) {
    let today = forecast
        .iter()
        .filter(|f| f.sunrise.map(|sr| sr <= now).unwrap_or(false))
        .max_by_key(|f| f.sunrise);

    let today = today.or_else(|| forecast.first());

    let tomorrow = today.and_then(|t| {
        let next_date = t.date.succ_opt();
        next_date.and_then(|d| forecast.iter().find(|f| f.date == d))
    });

    (today, tomorrow)
}

// ─────────────────────────────────────────────────────────────────────────
// Boucle principale.
// ─────────────────────────────────────────────────────────────────────────

pub async fn run(state: AppState) {
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_sunrise_seen: Option<DateTime<Utc>> = None;
    tracing::info!("Auto-switch loop démarrée (TICK = {:?})", TICK);

    loop {
        interval.tick().await;
        // Erreurs DB / I/O sont absorbées dans `tick_once` (warn log + continue).
        // Si un tick devait paniquer, tokio::spawn isole la task — la boucle s'arrêterait
        // mais sans crasher le process. La règle 1 (urgence) ne dépend que des sensors.
        tick_once(&state, &mut last_sunrise_seen).await;
    }
}

async fn tick_once(
    state: &AppState,
    last_sunrise_seen: &mut Option<DateTime<Utc>>,
) {
    let now = Utc::now();

    // Récupérer la prévision (peut être vide si DB down ou pas encore peuplé).
    let forecast = match state.db.as_ref() {
        Some(db) if db.is_connected() => {
            fetch_forecast_window(db).await.unwrap_or_else(|e| {
                tracing::warn!(error = %e, "fetch_forecast_window échoué");
                Vec::new()
            })
        }
        _ => Vec::new(),
    };

    let (today, tomorrow) = select_today_tomorrow(&forecast, now);
    let today_sunrise = today.and_then(|t| t.sunrise);
    let today_sunset = today.and_then(|t| t.sunset);
    let tomorrow_radiation = tomorrow.and_then(|t| t.shortwave_sum_kwh);

    // Snapshot des données + reset quotidien si on a passé sunrise.
    let (auto_snapshot, current_relay, voltage_inst, voltage_max5min) = {
        let mut inner = state.inner.lock();

        // Reset quotidien : on détecte le passage du sunrise du jour. Si on n'a
        // pas encore enregistré ce sunrise précis, on reset.
        if let Some(sr) = today_sunrise {
            let crossed = match *last_sunrise_seen {
                Some(prev) => prev != sr,
                None => true,
            };
            if crossed && now >= sr {
                if inner.auto.float_reached_today || inner.auto.eod_lockout {
                    tracing::info!("Reset quotidien (sunrise atteint)");
                }
                inner.auto.float_reached_today = false;
                inner.auto.eod_lockout = false;
                inner.auto.float_voltage_minutes = 0;
                *last_sunrise_seen = Some(sr);
            }
        }

        let v_inst = inner
            .sensors
            .iter()
            .find(|s| s.address == 0x40)
            .map(|s| s.bus_voltage_v);
        let v_max5 = inner.live.max_battery_voltage_recent(300);

        // Mettre à jour le SoC affiché (basé sur max 5 min, plus stable).
        if let Some(v) = v_max5.or(v_inst) {
            inner.auto.soc_percent = Some(soc_from_voltage(v));
        }

        // Compteurs de soutien (incrémentés par minute).
        let v_for_counter = v_max5.unwrap_or(0.0);
        if v_for_counter < V_LOW {
            inner.auto.low_voltage_minutes = inner.auto.low_voltage_minutes.saturating_add(1);
        } else {
            inner.auto.low_voltage_minutes = 0;
        }
        if v_for_counter >= V_RECOVER {
            inner.auto.recover_voltage_minutes =
                inner.auto.recover_voltage_minutes.saturating_add(1);
        } else {
            inner.auto.recover_voltage_minutes = 0;
        }
        if v_for_counter >= V_FLOAT {
            inner.auto.float_voltage_minutes =
                inner.auto.float_voltage_minutes.saturating_add(1);
            if inner.auto.float_voltage_minutes >= V_FLOAT_MIN_MINUTES
                && !inner.auto.float_reached_today
            {
                inner.auto.float_reached_today = true;
                tracing::info!(v = v_for_counter, "Float atteint aujourd'hui (≥10 min ≥ 27.2V)");
            }
        } else {
            inner.auto.float_voltage_minutes = 0;
        }

        (
            inner.auto.clone(),
            inner.published_state,
            v_inst,
            v_max5,
        )
    };

    let inputs = DecisionInputs {
        now,
        current_relay,
        voltage_inst,
        voltage_max5min,
        today_sunrise,
        today_sunset,
        tomorrow_radiation_kwh: tomorrow_radiation,
    };

    let decision = decide(&auto_snapshot, &inputs);

    // Persister la décision dans `inner` (toujours, même si Hold).
    {
        let mut inner = state.inner.lock();
        inner.auto.last_decision = Some(AutoDecision {
            at: now,
            reason: decision.reason.to_string(),
            message: decision.message.clone(),
        });
    }

    // Appliquer l'action.
    match decision.action {
        Action::Hold => {
            tracing::debug!(reason = decision.reason, "auto: hold");
        }
        Action::SwitchToGrid | Action::SwitchToSolar => {
            let target = match decision.action {
                Action::SwitchToGrid => Network::Grid,
                Action::SwitchToSolar => Network::Solar,
                Action::Hold => unreachable!(),
            };

            // Si la fenêtre EOD est en cours, on pose le verrou avant le switch
            // pour bloquer toute reprise SOLAR jusqu'au prochain sunrise.
            if decision.reason == "eod_recharge" {
                state.inner.lock().auto.eod_lockout = true;
            }

            // Acquérir le mutex relay (peut bloquer brièvement si watchdog/manuel
            // en cours — c'est OK).
            let mut relay = state.relay.lock().await;
            // Re-vérifier qu'on a toujours besoin du switch (un switch manuel a
            // pu se glisser entre temps).
            let actual = relay.current_state();
            let already_correct = matches!(
                (actual, target),
                (RelayState::Grid, Network::Grid) | (RelayState::Solar, Network::Solar)
            );
            if already_correct {
                tracing::debug!(?actual, ?target, "auto: déjà sur la bonne cible");
            } else {
                tracing::info!(
                    ?actual,
                    ?target,
                    reason = decision.reason,
                    "auto: switch déclenché"
                );
                let result = relay.switch_to(target, SWITCH_AUTO_DELAY).await;
                let new_state = relay.current_state();
                state.inner.lock().published_state = new_state;
                state.inner.lock().auto.last_switch_at = Some(now);
                match result {
                    Ok(()) => {
                        tracing::info!(?new_state, "auto: switch OK");
                        if let Some(db) = state.db.as_ref() {
                            if db.is_connected() {
                                let state_str = match new_state {
                                    RelayState::Open => "open",
                                    RelayState::Grid => "grid",
                                    RelayState::Solar => "solar",
                                };
                                let reason_str =
                                    format!("auto:{}", decision.reason);
                                if let Err(e) =
                                    log_relay_event(db, now, state_str, &reason_str).await
                                {
                                    tracing::warn!(error = %e, "log_relay_event échoué");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            "auto: switch ÉCHOUÉ — relais forcés ouverts"
                        );
                    }
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(h: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 27, h.try_into().unwrap(), 0, 0)
            .unwrap()
    }

    #[test]
    fn soc_at_table_points() {
        assert!((soc_from_voltage(23.6) - 0.0).abs() < 0.01);
        assert!((soc_from_voltage(24.4) - 30.0).abs() < 0.01);
        assert!((soc_from_voltage(24.8) - 50.0).abs() < 0.01);
        assert!((soc_from_voltage(25.0) - 60.0).abs() < 0.01);
        assert!((soc_from_voltage(25.6) - 75.0).abs() < 0.01);
        assert!((soc_from_voltage(26.4) - 90.0).abs() < 0.01);
        assert!((soc_from_voltage(27.2) - 100.0).abs() < 0.01);
    }

    #[test]
    fn soc_clamped_below_and_above() {
        assert!((soc_from_voltage(20.0) - 0.0).abs() < 0.01);
        assert!((soc_from_voltage(30.0) - 100.0).abs() < 0.01);
    }

    #[test]
    fn soc_interpolated_midpoints() {
        // Mid-point entre 25.0 (60%) et 25.6 (75%) → ~67.5%
        let s = soc_from_voltage(25.3);
        assert!((s - 67.5).abs() < 0.5);
    }

    #[test]
    fn eod_threshold_strong_sun_tomorrow() {
        assert!((eod_threshold(Some(5.0), true) - 26.3).abs() < 0.01);
    }

    #[test]
    fn eod_threshold_overcast_tomorrow() {
        assert!((eod_threshold(Some(1.0), true) - 27.0).abs() < 0.01);
    }

    #[test]
    fn eod_threshold_no_data_fallback() {
        assert!((eod_threshold(None, true) - 26.7).abs() < 0.01);
    }

    #[test]
    fn eod_threshold_bonus_if_float_not_reached() {
        assert!((eod_threshold(Some(5.0), false) - 26.5).abs() < 0.01);
    }

    fn base_inputs(v_inst: Option<f32>, v_max5: Option<f32>, current: RelayState) -> DecisionInputs {
        DecisionInputs {
            now: dt(12),
            current_relay: current,
            voltage_inst: v_inst,
            voltage_max5min: v_max5,
            today_sunrise: Some(dt(6)),
            today_sunset: Some(dt(20)),
            tomorrow_radiation_kwh: Some(3.0),
        }
    }

    #[test]
    fn rule1_emergency_switches_to_grid() {
        let auto = AutoState::default();
        let inputs = base_inputs(Some(24.0), Some(24.0), RelayState::Solar);
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::SwitchToGrid);
        assert_eq!(d.reason, "emergency_low_voltage");
    }

    #[test]
    fn rule1_emergency_works_even_when_disabled() {
        // Filet de sécurité : règle 1 doit court-circuiter le toggle.
        let mut auto = AutoState::default();
        auto.enabled = false;
        let inputs = base_inputs(Some(24.0), Some(24.0), RelayState::Solar);
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::SwitchToGrid);
    }

    #[test]
    fn auto_disabled_holds_outside_emergency() {
        let mut auto = AutoState::default();
        auto.enabled = false;
        let inputs = base_inputs(Some(26.0), Some(26.0), RelayState::Grid);
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::Hold);
        assert_eq!(d.reason, "auto_disabled");
    }

    #[test]
    fn rule2_eod_triggers_when_low_battery_late_day() {
        // 18h30 (sunset 20h, EOD = 18h)
        let mut inputs = base_inputs(Some(26.5), Some(26.5), RelayState::Solar);
        inputs.now = dt(19);
        let auto = AutoState::default();
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::SwitchToGrid);
        assert_eq!(d.reason, "eod_recharge");
    }

    #[test]
    fn rule2_eod_no_trigger_if_battery_full() {
        let mut inputs = base_inputs(Some(27.3), Some(27.3), RelayState::Solar);
        inputs.now = dt(19);
        let auto = AutoState::default();
        let d = decide(&auto, &inputs);
        assert_ne!(d.reason, "eod_recharge");
    }

    #[test]
    fn rule2_eod_no_trigger_if_lockout_already() {
        let mut inputs = base_inputs(Some(26.5), Some(26.5), RelayState::Grid);
        inputs.now = dt(19);
        let mut auto = AutoState::default();
        auto.eod_lockout = true;
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::Hold);
        assert_eq!(d.reason, "hold");
    }

    #[test]
    fn rule3_voltage_low_sustained_switches_after_3min() {
        let inputs = base_inputs(Some(25.0), Some(25.0), RelayState::Solar);
        let mut auto = AutoState::default();
        auto.low_voltage_minutes = 3;
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::SwitchToGrid);
        assert_eq!(d.reason, "voltage_low_sustained");
    }

    #[test]
    fn rule3_no_switch_below_min_minutes() {
        let inputs = base_inputs(Some(25.0), Some(25.0), RelayState::Solar);
        let mut auto = AutoState::default();
        auto.low_voltage_minutes = 2;
        let d = decide(&auto, &inputs);
        assert_eq!(d.reason, "hold");
    }

    #[test]
    fn rule4_voltage_recovered_allows_solar() {
        let inputs = base_inputs(Some(26.5), Some(26.5), RelayState::Grid);
        let mut auto = AutoState::default();
        auto.recover_voltage_minutes = 5;
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::SwitchToSolar);
        assert_eq!(d.reason, "voltage_recovered");
    }

    #[test]
    fn rule4_blocked_by_eod_lockout() {
        let inputs = base_inputs(Some(26.5), Some(26.5), RelayState::Grid);
        let mut auto = AutoState::default();
        auto.recover_voltage_minutes = 5;
        auto.eod_lockout = true;
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::Hold);
    }

    #[test]
    fn rule4_blocked_outside_solar_window() {
        // 22h, après sunset
        let mut inputs = base_inputs(Some(26.5), Some(26.5), RelayState::Grid);
        inputs.now = dt(22);
        let mut auto = AutoState::default();
        auto.recover_voltage_minutes = 5;
        let d = decide(&auto, &inputs);
        assert_ne!(d.action, Action::SwitchToSolar);
    }

    #[test]
    fn anti_oscillation_blocks_recent_switch() {
        let mut inputs = base_inputs(Some(25.0), Some(25.0), RelayState::Solar);
        inputs.now = dt(12);
        let mut auto = AutoState::default();
        auto.low_voltage_minutes = 5;
        auto.last_switch_at = Some(dt(12) - chrono::Duration::minutes(2));
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::Hold);
        assert_eq!(d.reason, "anti_oscillation");
    }

    #[test]
    fn anti_oscillation_does_not_block_emergency() {
        let mut inputs = base_inputs(Some(24.0), Some(24.0), RelayState::Solar);
        inputs.now = dt(12);
        let mut auto = AutoState::default();
        auto.last_switch_at = Some(dt(12) - chrono::Duration::minutes(1));
        let d = decide(&auto, &inputs);
        assert_eq!(d.action, Action::SwitchToGrid);
        assert_eq!(d.reason, "emergency_low_voltage");
    }
}
