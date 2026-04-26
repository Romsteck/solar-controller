use rppal::gpio::{Gpio, OutputPin};
use std::time::Duration;
use thiserror::Error;
use crate::state::Network;

const PIN_GRID: u8 = 20;
const PIN_SOLAR: u8 = 26;

/// Délai mécanique minimum entre l'ouverture d'un relais et la fermeture de
/// l'autre. Marge ×33 sur un release time typique de 10-15 ms. JAMAIS descendre
/// en dessous : c'est ce qui empêche les deux contacts d'être physiquement
/// fermés simultanément (court-circuit grid+solaire = arc, incendie, mort).
pub const RELAY_SETTLE_MIN: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayState {
    Open,
    Grid,
    Solar,
}

impl RelayState {
    /// Cible par défaut pour un toggle. `Open` part vers Grid (état nominal au boot).
    pub fn next_target(self) -> Network {
        match self {
            RelayState::Open | RelayState::Solar => Network::Grid,
            RelayState::Grid => Network::Solar,
        }
    }
}

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("CATASTROPHIQUE: les deux relais commandés fermés simultanément (grid_low={grid_low}, solar_low={solar_low})")]
    BothClosed { grid_low: bool, solar_low: bool },

    #[error("incohérence GPIO/état logiciel: state={expected:?} mais grid_low={grid_low} solar_low={solar_low}")]
    StateMismatch {
        expected: RelayState,
        grid_low: bool,
        solar_low: bool,
    },
}

pub struct RelayController {
    grid: OutputPin,
    solar: OutputPin,
    state: RelayState,
}

// HAT actif-LOW : LOW = relais fermé (contact établi), HIGH = ouvert.
// Tout passage de bas niveau passe par ces helpers pour clarifier l'intention.
fn drive_low(pin: &mut OutputPin) { pin.set_low(); }
fn drive_high(pin: &mut OutputPin) { pin.set_high(); }

impl RelayController {
    pub fn new() -> anyhow::Result<Self> {
        let gpio = Gpio::new()?;
        // `into_output_high` initialise le pin en sortie ET au niveau HIGH de
        // façon atomique. `into_output()` hérite du niveau précédent du registre,
        // ce qui peut laisser un pin en LOW si le process précédent a crashé
        // avec un relais fermé (scénario S3).
        let mut grid = gpio.get(PIN_GRID)?.into_output_high();
        let mut solar = gpio.get(PIN_SOLAR)?.into_output_high();
        // Si le process meurt sans appeler Drop (SIGKILL, panic non rattrapé,
        // OOM kill), rppal libère les pins en mode input. Pour un HAT actif-LOW
        // typique, le pull-up tire HIGH → relais ouvert.
        grid.set_reset_on_drop(true);
        solar.set_reset_on_drop(true);
        Ok(Self {
            grid,
            solar,
            state: RelayState::Open,
        })
    }

    pub fn current_state(&self) -> RelayState {
        self.state
    }

    /// Force les deux relais en position OUVERTE. Jamais bloquant, ne panique pas.
    pub fn open_all(&mut self) {
        drive_high(&mut self.grid);
        drive_high(&mut self.solar);
        self.state = RelayState::Open;
    }

    /// Bascule vers `target` avec break-before-make et délai mécanique.
    ///
    /// Séquence non-contournable :
    ///   1. `open_all()` (les deux relais HIGH).
    ///   2. Sleep `max(settle, RELAY_SETTLE_MIN)`.
    ///   3. Vérifie via GPIO que les deux pins sont bien HIGH.
    ///   4. Ferme uniquement le relais cible.
    ///   5. Vérifie l'état final.
    ///
    /// `&mut self` garantit l'exclusion mutuelle au compile-time.
    pub async fn switch_to(
        &mut self,
        target: Network,
        settle: Duration,
    ) -> Result<(), RelayError> {
        // Étape 1 : ouvrir tout, INCONDITIONNELLEMENT.
        self.open_all();

        // Étape 2 : attendre le release time mécanique.
        let actual_settle = settle.max(RELAY_SETTLE_MIN);
        tokio::time::sleep(actual_settle).await;

        // Étape 3 : sanity-check des pins post-sleep.
        if self.grid.is_set_low() || self.solar.is_set_low() {
            self.open_all();
            return Err(RelayError::StateMismatch {
                expected: RelayState::Open,
                grid_low: self.grid.is_set_low(),
                solar_low: self.solar.is_set_low(),
            });
        }

        // Étape 4 : fermer UNIQUEMENT le pin cible.
        match target {
            Network::Grid => {
                drive_low(&mut self.grid);
                self.state = RelayState::Grid;
            }
            Network::Solar => {
                drive_low(&mut self.solar);
                self.state = RelayState::Solar;
            }
        }

        // Étape 5 : vérification finale (lit le registre GPIO).
        self.verify()
    }

    /// Relit l'état GPIO et vérifie la cohérence avec le state interne.
    /// Si les deux pins sont LOW (catastrophique), force `open_all` et retourne `BothClosed`.
    pub fn verify(&mut self) -> Result<(), RelayError> {
        let grid_low = self.grid.is_set_low();
        let solar_low = self.solar.is_set_low();

        if grid_low && solar_low {
            self.open_all();
            return Err(RelayError::BothClosed { grid_low, solar_low });
        }

        let observed = match (grid_low, solar_low) {
            (false, false) => RelayState::Open,
            (true, false) => RelayState::Grid,
            (false, true) => RelayState::Solar,
            (true, true) => unreachable!("écarté par le check précédent"),
        };

        if observed != self.state {
            let expected = self.state;
            self.open_all();
            return Err(RelayError::StateMismatch {
                expected,
                grid_low,
                solar_low,
            });
        }
        Ok(())
    }
}

impl Drop for RelayController {
    fn drop(&mut self) {
        // Dernier filet de sécurité au démantèlement.
        drive_high(&mut self.grid);
        drive_high(&mut self.solar);
        self.state = RelayState::Open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests de pure logique sur la machine d'état. Les tests qui exercent les
    // GPIO réels nécessitent un Pi (rppal::Gpio::new() échoue hors hardware) ;
    // une abstraction par trait serait nécessaire — hors scope.

    #[test]
    fn next_target_from_grid_is_solar() {
        assert_eq!(RelayState::Grid.next_target(), Network::Solar);
    }

    #[test]
    fn next_target_from_solar_is_grid() {
        assert_eq!(RelayState::Solar.next_target(), Network::Grid);
    }

    #[test]
    fn next_target_from_open_is_grid() {
        // INVARIANT : depuis l'état sûr (Open), on retourne au mode nominal Grid.
        assert_eq!(RelayState::Open.next_target(), Network::Grid);
    }

    #[test]
    fn relay_settle_min_is_at_least_500ms() {
        // INVARIANT : ne JAMAIS descendre en dessous de 500 ms. Cette constante
        // est ce qui empêche la fermeture simultanée des deux relais lors d'un
        // break-before-make. Si quelqu'un baisse cette valeur, ce test cassera.
        assert!(RELAY_SETTLE_MIN.as_millis() >= 500);
    }
}
