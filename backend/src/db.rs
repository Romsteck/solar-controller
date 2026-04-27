use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

/// Pool PostgreSQL + flag de connectivité partagé.
///
/// Le flag `connected` est lu par `/api/status` (sans bloquer) et mis à jour
/// par `health_loop`. La transition est loggée une fois (pas de spam).
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
    connected: Arc<AtomicBool>,
}

impl Db {
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    fn set_connected(&self, value: bool) {
        let prev = self.connected.swap(value, Ordering::Relaxed);
        if prev != value {
            if value {
                tracing::info!("DB reconnectée");
            } else {
                tracing::warn!("DB déconnectée");
            }
        }
    }
}

/// Tente de se connecter à la DB avec un retry borné.
/// Max ~21s : 3 essais × (5s connect_timeout + 2s sleep entre essais).
/// Au succès, applique les migrations idempotentes.
/// Au cumul de tous les échecs, retourne None (le service tourne en mode dégradé).
pub async fn connect_with_retry(url: &str) -> Option<Db> {
    const MAX_ATTEMPTS: u32 = 3;
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    const RETRY_DELAY: Duration = Duration::from_secs(2);

    for attempt in 1..=MAX_ATTEMPTS {
        match PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(CONNECT_TIMEOUT)
            .connect(url)
            .await
        {
            Ok(pool) => {
                if let Err(e) = sqlx::query("SELECT 1").execute(&pool).await {
                    tracing::warn!(error = %e, attempt, "DB ping a échoué après connect");
                    if attempt < MAX_ATTEMPTS {
                        tokio::time::sleep(RETRY_DELAY).await;
                        continue;
                    }
                    return None;
                }

                let db = Db {
                    pool,
                    connected: Arc::new(AtomicBool::new(true)),
                };

                if let Err(e) = run_migrations(&db).await {
                    tracing::error!(error = %e, "Migrations DB échouées — service en mode dégradé");
                    return None;
                }

                tracing::info!("DB connectée (essai {}/{})", attempt, MAX_ATTEMPTS);
                return Some(db);
            }
            Err(e) => {
                tracing::warn!(error = %e, attempt, max = MAX_ATTEMPTS, "Connexion DB échouée");
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }
        }
    }

    tracing::warn!("DB injoignable après {} essais — mode dégradé", MAX_ATTEMPTS);
    None
}

async fn run_migrations(db: &Db) -> anyhow::Result<()> {
    let sql = include_str!("../migrations/001_init.sql");
    sqlx::raw_sql(sql).execute(&db.pool).await?;
    Ok(())
}

/// Boucle de health-check : ping toutes les 60s, met à jour `connected`.
/// Ne logge que les transitions (set_connected interne).
pub async fn health_loop(db: Db) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await; // skip le tick immédiat (juste après connect_with_retry)

    loop {
        interval.tick().await;
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            sqlx::query("SELECT 1").execute(db.pool()),
        )
        .await;
        let ok = matches!(result, Ok(Ok(_)));
        db.set_connected(ok);
    }
}
