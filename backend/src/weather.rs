use std::time::Duration;
use chrono::Utc;
use serde::Deserialize;
use crate::db::Db;

const TICK: Duration = Duration::from_secs(15 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const SOURCE: &str = "open-meteo";

#[derive(Deserialize)]
struct OpenMeteoResponse {
    current: Option<OpenMeteoCurrent>,
}

#[derive(Deserialize)]
struct OpenMeteoCurrent {
    temperature_2m: Option<f32>,
    cloud_cover: Option<f32>,
    shortwave_radiation: Option<f32>,
}

/// Fetch toutes les 15 min depuis Open-Meteo et persiste dans `weather_samples`.
/// Erreurs HTTP loggées seulement à la transition (pas de spam).
pub async fn weather_loop(db: Db, lat: f32, lon: f32) {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={lat}&longitude={lon}\
         &current=temperature_2m,cloud_cover,shortwave_radiation"
    );

    let client = match reqwest::Client::builder().timeout(HTTP_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Init reqwest::Client échoué — weather_loop annulée");
            return;
        }
    };

    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_failed = false;

    loop {
        interval.tick().await;
        match fetch_and_insert(&client, &url, &db).await {
            Ok(()) => {
                if last_failed {
                    tracing::info!("Météo Open-Meteo récupérée (récupération)");
                    last_failed = false;
                }
            }
            Err(e) => {
                if !last_failed {
                    tracing::warn!(error = %e, "Fetch météo Open-Meteo échoué");
                    last_failed = true;
                }
            }
        }
    }
}

async fn fetch_and_insert(client: &reqwest::Client, url: &str, db: &Db) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let body: OpenMeteoResponse = resp.json().await?;
    let cur = body
        .current
        .ok_or_else(|| anyhow::anyhow!("réponse Open-Meteo sans champ 'current'"))?;

    sqlx::query(
        "INSERT INTO weather_samples (ts, temp_c, cloud_cover_pct, shortwave_wm2, source)
         VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (ts) DO NOTHING",
    )
    .bind(Utc::now())
    .bind(cur.temperature_2m)
    .bind(cur.cloud_cover)
    .bind(cur.shortwave_radiation)
    .bind(SOURCE)
    .execute(db.pool())
    .await?;
    Ok(())
}
