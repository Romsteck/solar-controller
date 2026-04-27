use std::time::Duration;
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Deserialize;
use crate::db::Db;

const TICK_CURRENT: Duration = Duration::from_secs(15 * 60);
const TICK_DAILY: Duration = Duration::from_secs(60 * 60);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const SOURCE: &str = "open-meteo";

#[derive(Deserialize)]
struct OpenMeteoCurrentResponse {
    current: Option<OpenMeteoCurrent>,
}

#[derive(Deserialize)]
struct OpenMeteoCurrent {
    temperature_2m: Option<f32>,
    cloud_cover: Option<f32>,
    shortwave_radiation: Option<f32>,
}

#[derive(Deserialize)]
struct OpenMeteoDailyResponse {
    daily: Option<OpenMeteoDaily>,
}

#[derive(Deserialize)]
struct OpenMeteoDaily {
    /// Une chaîne par jour, ISO date locale (timezone=auto fournit du local).
    time: Vec<String>,
    sunrise: Vec<Option<String>>,
    sunset: Vec<Option<String>>,
    /// MJ/m² par jour (Open-Meteo). On convertit en kWh/m² (÷ 3.6) à l'insertion.
    shortwave_radiation_sum: Vec<Option<f32>>,
}

/// Boucle météo : deux requêtes parallèles (current 15 min, daily 1 h) sur la
/// même API Open-Meteo. Erreurs HTTP loggées seulement à la transition.
pub async fn weather_loop(db: Db, lat: f32, lon: f32) {
    let client = match reqwest::Client::builder().timeout(HTTP_TIMEOUT).build() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Init reqwest::Client échoué — weather_loop annulée");
            return;
        }
    };

    let current_url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={lat}&longitude={lon}\
         &current=temperature_2m,cloud_cover,shortwave_radiation"
    );
    let daily_url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={lat}&longitude={lon}\
         &daily=sunrise,sunset,shortwave_radiation_sum\
         &forecast_days=3&past_days=1&timezone=auto"
    );

    // Premier fetch immédiat des deux flux (ne pas attendre 15 min après le boot).
    if let Err(e) = fetch_and_insert_current(&client, &current_url, &db).await {
        tracing::warn!(error = %e, "Fetch météo current initial échoué");
    }
    if let Err(e) = fetch_and_insert_daily(&client, &daily_url, &db).await {
        tracing::warn!(error = %e, "Fetch météo daily initial échoué");
    }

    tokio::join!(
        current_subloop(client.clone(), current_url, db.clone()),
        daily_subloop(client.clone(), daily_url, db.clone()),
    );
}

async fn current_subloop(client: reqwest::Client, url: String, db: Db) {
    let mut interval = tokio::time::interval(TICK_CURRENT);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await; // skip immédiat (déjà fetché en init)
    let mut last_failed = false;
    loop {
        interval.tick().await;
        match fetch_and_insert_current(&client, &url, &db).await {
            Ok(()) => {
                if last_failed {
                    tracing::info!("Météo current récupérée");
                    last_failed = false;
                }
            }
            Err(e) => {
                if !last_failed {
                    tracing::warn!(error = %e, "Fetch météo current échoué");
                    last_failed = true;
                }
            }
        }
    }
}

async fn daily_subloop(client: reqwest::Client, url: String, db: Db) {
    let mut interval = tokio::time::interval(TICK_DAILY);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval.tick().await; // skip immédiat (déjà fetché en init)
    let mut last_failed = false;
    loop {
        interval.tick().await;
        match fetch_and_insert_daily(&client, &url, &db).await {
            Ok(()) => {
                if last_failed {
                    tracing::info!("Météo daily récupérée");
                    last_failed = false;
                }
            }
            Err(e) => {
                if !last_failed {
                    tracing::warn!(error = %e, "Fetch météo daily échoué");
                    last_failed = true;
                }
            }
        }
    }
}

async fn fetch_and_insert_current(
    client: &reqwest::Client,
    url: &str,
    db: &Db,
) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let body: OpenMeteoCurrentResponse = resp.json().await?;
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

async fn fetch_and_insert_daily(
    client: &reqwest::Client,
    url: &str,
    db: &Db,
) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?.error_for_status()?;
    let body: OpenMeteoDailyResponse = resp.json().await?;
    let daily = body
        .daily
        .ok_or_else(|| anyhow::anyhow!("réponse Open-Meteo sans champ 'daily'"))?;

    let n = daily.time.len();
    if daily.sunrise.len() != n
        || daily.sunset.len() != n
        || daily.shortwave_radiation_sum.len() != n
    {
        return Err(anyhow::anyhow!(
            "tailles daily incohérentes: time={}, sunrise={}, sunset={}, sum={}",
            n,
            daily.sunrise.len(),
            daily.sunset.len(),
            daily.shortwave_radiation_sum.len()
        ));
    }

    for i in 0..n {
        let date = match NaiveDate::parse_from_str(&daily.time[i], "%Y-%m-%d") {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(error = %e, raw = daily.time[i].as_str(), "date daily parse échouée");
                continue;
            }
        };
        let sunrise = daily.sunrise[i].as_deref().and_then(parse_local_iso);
        let sunset = daily.sunset[i].as_deref().and_then(parse_local_iso);
        // MJ/m² → kWh/m² (1 kWh = 3.6 MJ).
        let kwh = daily.shortwave_radiation_sum[i].map(|mj| mj / 3.6);

        sqlx::query(
            "INSERT INTO forecast_daily (date, sunrise, sunset, shortwave_sum_kwh, fetched_at)
             VALUES ($1,$2,$3,$4, now())
             ON CONFLICT (date) DO UPDATE
             SET sunrise = EXCLUDED.sunrise,
                 sunset = EXCLUDED.sunset,
                 shortwave_sum_kwh = EXCLUDED.shortwave_sum_kwh,
                 fetched_at = now()",
        )
        .bind(date)
        .bind(sunrise)
        .bind(sunset)
        .bind(kwh)
        .execute(db.pool())
        .await?;
    }

    Ok(())
}

/// Parse un timestamp Open-Meteo `&timezone=auto` (ex: `2026-04-27T06:48`).
/// Open-Meteo ne fournit pas l'offset dans la chaîne — il l'envoie séparément
/// via `utc_offset_seconds`. Pour rester simple, on parse comme heure locale
/// et on suppose Brussels (UTC+1 hiver / +2 été) via heuristique date.
/// Approximation suffisante ici (la décision EOD s'enclenche à `sunset - 2h`,
/// donc une erreur de ±1h de DST est absorbée par la marge).
fn parse_local_iso(s: &str) -> Option<DateTime<Utc>> {
    // Open-Meteo renvoie soit "YYYY-MM-DDTHH:MM" soit avec offset complet.
    // On essaie plusieurs formats.
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M") {
        // Brussels DST : heuristique simple (DST actif ~ dernier dim mars → dernier dim oct).
        let month = naive.date().month0() + 1;
        let offset_hours = if (4..=9).contains(&month) { 2 } else { 1 };
        let utc_naive = naive - chrono::Duration::hours(offset_hours);
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(utc_naive, Utc));
    }
    if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        let month = naive.date().month0() + 1;
        let offset_hours = if (4..=9).contains(&month) { 2 } else { 1 };
        let utc_naive = naive - chrono::Duration::hours(offset_hours);
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(utc_naive, Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso_with_offset() {
        let dt = parse_local_iso("2026-04-27T06:48:00+02:00").unwrap();
        assert_eq!(dt.to_rfc3339(), "2026-04-27T04:48:00+00:00");
    }

    #[test]
    fn parse_iso_local_summer_brussels() {
        // Été (avril–septembre) → UTC+2.
        let dt = parse_local_iso("2026-07-15T20:30").unwrap();
        // 20:30 local été → 18:30 UTC.
        assert_eq!(dt.format("%H:%M").to_string(), "18:30");
    }

    #[test]
    fn parse_iso_local_winter_brussels() {
        // Hiver (oct–mars) → UTC+1.
        let dt = parse_local_iso("2026-12-15T17:00").unwrap();
        assert_eq!(dt.format("%H:%M").to_string(), "16:00");
    }
}
