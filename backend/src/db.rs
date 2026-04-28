use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use chrono::{DateTime, NaiveDate, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};

/// Pool PostgreSQL + flags partagés.
///
/// `connected` est lu par `/api/status` (sans bloquer) et mis à jour par
/// `health_loop`. `schema_initialized` empêche de relancer les migrations à
/// chaque reconnexion (one-shot, idempotent quand même côté SQL).
#[derive(Clone)]
pub struct Db {
    pool: PgPool,
    connected: Arc<AtomicBool>,
    schema_initialized: Arc<AtomicBool>,
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
                tracing::info!("DB connectée");
            } else {
                tracing::warn!("DB déconnectée");
            }
        }
    }
}

/// Construit un pool DB en mode "lazy" : aucun socket n'est ouvert tant qu'on
/// ne fait pas de query. Permet au service de démarrer même si la DB est down
/// au boot — `health_loop` se chargera de retenter régulièrement.
///
/// Échoue uniquement si l'URL est syntaxiquement invalide.
pub fn connect_lazy(url: &str) -> Result<Db, sqlx::Error> {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(url)?;
    Ok(Db {
        pool,
        connected: Arc::new(AtomicBool::new(false)),
        schema_initialized: Arc::new(AtomicBool::new(false)),
    })
}

async fn run_migrations(db: &Db) -> anyhow::Result<()> {
    let sql = include_str!("../migrations/001_init.sql");
    sqlx::raw_sql(sql).execute(&db.pool).await?;
    Ok(())
}

/// Tente un ping + (si nécessaire) les migrations. Met à jour le flag
/// `connected` selon le résultat. Retourne `true` si la DB répond ET que le
/// schéma est initialisé.
pub async fn try_connect_and_init(db: &Db) -> bool {
    // Ping avec timeout court — ne bloque pas la boucle si la DB répond mal.
    let ping_ok = matches!(
        tokio::time::timeout(
            Duration::from_secs(3),
            sqlx::query("SELECT 1").execute(&db.pool),
        )
        .await,
        Ok(Ok(_))
    );
    if !ping_ok {
        db.set_connected(false);
        return false;
    }

    // Init schema (one-shot, mais SQL idempotent → safe si on retry).
    if !db.schema_initialized.load(Ordering::Relaxed) {
        match tokio::time::timeout(Duration::from_secs(10), run_migrations(db)).await {
            Ok(Ok(())) => {
                db.schema_initialized.store(true, Ordering::Relaxed);
                tracing::info!("Schéma DB initialisé");
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, "Migrations DB échouées");
                db.set_connected(false);
                return false;
            }
            Err(_) => {
                tracing::error!("Migrations DB timeout");
                db.set_connected(false);
                return false;
            }
        }
    }

    db.set_connected(true);
    true
}

/// Boucle de santé : retente toutes les 10s. Sert à la fois de monitoring
/// (transition logguée via `set_connected`) et de reconnect — sqlx réouvrira
/// les sockets sous-jacents automatiquement, on a juste besoin de pinger pour
/// déclencher / vérifier ça.
pub async fn health_loop(db: Db) {
    const TICK: Duration = Duration::from_secs(10);
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await; // skip le tick immédiat (le boot a déjà tenté un ping)

    loop {
        interval.tick().await;
        try_connect_and_init(&db).await;
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
