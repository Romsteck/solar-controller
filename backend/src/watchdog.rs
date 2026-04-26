use std::time::Duration;
use crate::state::AppState;

const WATCHDOG_INTERVAL: Duration = Duration::from_millis(500);

/// Tâche périodique qui relit l'état physique des GPIO et vérifie qu'il
/// correspond au state interne du `RelayController`.
///
/// Si une incohérence est détectée — y compris l'état catastrophique « les
/// deux relais commandés fermés » — `verify` force `open_all` et retourne une
/// erreur. Cette tâche logge et republie le state.
///
/// `try_lock` plutôt que `lock` : le watchdog ne doit jamais bloquer un switch
/// en cours. Si le mutex est déjà tenu (switch_to en cours, qui suit son propre
/// chemin sécurisé), on saute ce tick et on revérifiera au suivant.
pub async fn run(state: AppState) {
    let mut interval = tokio::time::interval(WATCHDOG_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let Ok(mut relay) = state.relay.try_lock() else {
            // Switch en cours — switch_to est responsable de la cohérence.
            continue;
        };
        match relay.verify() {
            Ok(()) => { /* tout va bien */ }
            Err(e) => {
                // `verify` a déjà appelé `open_all` en cas d'incohérence.
                tracing::error!(error = %e, "WATCHDOG : incohérence détectée, relais forcés ouverts");
                // Republier l'état (forcément Open après open_all).
                let new_state = relay.current_state();
                state.inner.lock().published_state = new_state;
            }
        }
    }
}
