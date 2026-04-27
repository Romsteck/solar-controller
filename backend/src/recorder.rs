use std::time::Duration;
use chrono::Utc;
use crate::db::Db;
use crate::state::{AppState, SensorReading, UpsReading};

const TICK: Duration = Duration::from_secs(10);

/// Snapshot l'état toutes les 10s et l'INSERT en batch dans PostgreSQL.
/// La loop ne meurt jamais : toute erreur SQL est loggée en warn et la loop continue.
pub async fn record_loop(state: AppState, db: Db) {
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // skip le premier tick immédiat : la loop sensors n'a peut-être pas encore tourné
    interval.tick().await;

    loop {
        interval.tick().await;

        let (sensors, ups) = {
            let inner = state.inner.lock();
            (inner.sensors.clone(), inner.ups.clone())
        };

        let now = Utc::now();

        if !sensors.is_empty() {
            if let Err(e) = insert_sensors(&db, now, &sensors).await {
                tracing::warn!(error = %e, "INSERT sensor_samples échoué");
            }
        }

        if let Some(u) = ups.as_ref() {
            if let Err(e) = insert_ups(&db, now, u).await {
                tracing::warn!(error = %e, "INSERT ups_samples échoué");
            }
        }
    }
}

async fn insert_sensors(db: &Db, ts: chrono::DateTime<Utc>, sensors: &[SensorReading]) -> Result<(), sqlx::Error> {
    // Multi-row INSERT pour économiser les round-trips. Conflict ts+address → DO NOTHING
    // (on ne devrait pas en avoir, mais ça protège contre un double-tick théorique).
    let mut qb = sqlx::QueryBuilder::new(
        "INSERT INTO sensor_samples (ts, address, bus_v, current_ma) ",
    );
    qb.push_values(sensors.iter(), |mut b, s| {
        b.push_bind(ts)
            .push_bind(s.address as i16)
            .push_bind(s.bus_voltage_v)
            .push_bind(s.current_ma);
    });
    qb.push(" ON CONFLICT (ts, address) DO NOTHING");
    qb.build().execute(db.pool()).await?;
    Ok(())
}

async fn insert_ups(db: &Db, ts: chrono::DateTime<Utc>, u: &UpsReading) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ups_samples
         (ts, input_v, input_hz, output_v, load_pct, battery_pct, battery_v, runtime_s, status)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT (ts) DO NOTHING",
    )
    .bind(ts)
    .bind(u.input_voltage_v)
    .bind(u.input_frequency_hz)
    .bind(u.output_voltage_v)
    .bind(u.load_pct)
    .bind(u.battery_pct)
    .bind(u.battery_voltage_v)
    .bind(u.runtime_s.map(|v| v as i32))
    .bind(u.status.as_deref())
    .execute(db.pool())
    .await?;
    Ok(())
}
