use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

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

// ─────────────────────────────────────────────────────────────────────────
// Settings (clé/valeur persistantes — auto_enabled, etc.)
// ─────────────────────────────────────────────────────────────────────────

/// Lit un booléen depuis la table `settings`. Retourne `default` si la clé est
/// absente, illisible, ou si la DB renvoie une erreur (mode dégradé).
pub async fn get_setting_bool(db: &Db, key: &str, default: bool) -> bool {
    match sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = $1")
        .bind(key)
        .fetch_optional(db.pool())
        .await
    {
        Ok(Some(v)) => match v.as_str() {
            "true" | "1" => true,
            "false" | "0" => false,
            other => {
                tracing::warn!(key, value = other, "valeur settings non-bool, fallback default");
                default
            }
        },
        Ok(None) => default,
        Err(e) => {
            tracing::warn!(error = %e, key, "lecture settings échouée, fallback default");
            default
        }
    }
}

/// Écrit un booléen dans la table `settings` (UPSERT).
pub async fn set_setting_bool(db: &Db, key: &str, value: bool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at)
         VALUES ($1, $2, now())
         ON CONFLICT (key) DO UPDATE
         SET value = EXCLUDED.value, updated_at = now()",
    )
    .bind(key)
    .bind(if value { "true" } else { "false" })
    .execute(db.pool())
    .await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// relay_events : audit trail des switchs (manuel + auto + watchdog)
// ─────────────────────────────────────────────────────────────────────────

pub async fn log_relay_event(
    db: &Db,
    ts: DateTime<Utc>,
    state: &str,
    reason: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO relay_events (ts, state, reason)
         VALUES ($1, $2, $3)
         ON CONFLICT (ts) DO NOTHING",
    )
    .bind(ts)
    .bind(state)
    .bind(reason)
    .execute(db.pool())
    .await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// forecast_daily : prévisions journalières (sunrise/sunset/radiation)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ForecastDay {
    pub date: NaiveDate,
    pub sunrise: Option<DateTime<Utc>>,
    pub sunset: Option<DateTime<Utc>>,
    pub shortwave_sum_kwh: Option<f32>,
}

/// Récupère les prévisions de la fenêtre [hier, aujourd'hui, demain].
/// Utilisé par la boucle auto pour déterminer "today" et "tomorrow"
/// indépendamment du fuseau horaire de la session PostgreSQL.
pub async fn fetch_forecast_window(db: &Db) -> Result<Vec<ForecastDay>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT date, sunrise, sunset, shortwave_sum_kwh
         FROM forecast_daily
         WHERE date >= CURRENT_DATE - INTERVAL '1 day'
           AND date <= CURRENT_DATE + INTERVAL '2 days'
         ORDER BY date ASC",
    )
    .fetch_all(db.pool())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(ForecastDay {
            date: row.try_get("date")?,
            sunrise: row.try_get("sunrise")?,
            sunset: row.try_get("sunset")?,
            shortwave_sum_kwh: row.try_get("shortwave_sum_kwh")?,
        });
    }
    Ok(out)
}
