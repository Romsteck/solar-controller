use std::str::FromStr;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::Row;
use crate::db::Db;

#[derive(Debug, Clone, Copy)]
pub enum Range {
    Hour,
    Day,
    Week,
    Month,
}

impl FromStr for Range {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "hour" => Ok(Range::Hour),
            "day" => Ok(Range::Day),
            "week" => Ok(Range::Week),
            "month" => Ok(Range::Month),
            _ => Err(()),
        }
    }
}

impl Range {
    /// (interval SQL pour la fenêtre, taille du bucket pour date_bin)
    fn bounds(self) -> (&'static str, &'static str) {
        match self {
            Range::Hour => ("1 hour", "1 minute"),
            Range::Day => ("1 day", "5 minutes"),
            Range::Week => ("7 days", "1 hour"),
            Range::Month => ("30 days", "6 hours"),
        }
    }
}

/// Réponse orientée série pour minimiser les bytes.
/// Chaque tableau a la même longueur que `ts`.
#[derive(Debug, Serialize)]
pub struct HistoryPayload {
    pub range: &'static str,
    pub bucket: &'static str,
    pub ts: Vec<i64>,
    pub sensor_grid_v: Vec<Option<f32>>,
    pub sensor_grid_ma: Vec<Option<f32>>,
    pub sensor_solar_v: Vec<Option<f32>>,
    pub sensor_solar_ma: Vec<Option<f32>>,
    pub ups_input_v: Vec<Option<f32>>,
    pub ups_battery_v: Vec<Option<f32>>,
    pub weather_temp_c: Vec<Option<f32>>,
    pub weather_cloud_pct: Vec<Option<f32>>,
    pub weather_radiation: Vec<Option<f32>>,
}

/// Une seule query côté DB (10.0.0.20) qui agrège tout sur l'axe des buckets.
/// La Pi ne reçoit que le résultat agrégé (au plus quelques centaines de lignes).
pub async fn fetch_history(db: &Db, range: Range) -> Result<HistoryPayload, sqlx::Error> {
    let (window, bucket) = range.bounds();

    let sql = format!(
        r#"
WITH params AS (
    SELECT
        now() - INTERVAL '{window}' AS since,
        INTERVAL '{bucket}' AS bsize
),
axis AS (
    SELECT date_bin((SELECT bsize FROM params), g, TIMESTAMPTZ 'epoch') AS ts
    FROM (SELECT (SELECT since FROM params) AS s, now() AS e) p,
    LATERAL generate_series(p.s, p.e, (SELECT bsize FROM params)) g
    GROUP BY 1
),
s40 AS (
    SELECT date_bin((SELECT bsize FROM params), ts, TIMESTAMPTZ 'epoch') AS b,
           AVG(bus_v)::REAL AS v, AVG(current_ma)::REAL AS ma
    FROM sensor_samples
    WHERE ts > (SELECT since FROM params) AND address = 64
    GROUP BY b
),
s41 AS (
    SELECT date_bin((SELECT bsize FROM params), ts, TIMESTAMPTZ 'epoch') AS b,
           AVG(bus_v)::REAL AS v, AVG(current_ma)::REAL AS ma
    FROM sensor_samples
    WHERE ts > (SELECT since FROM params) AND address = 65
    GROUP BY b
),
u AS (
    SELECT date_bin((SELECT bsize FROM params), ts, TIMESTAMPTZ 'epoch') AS b,
           AVG(input_v)::REAL AS input_v, AVG(battery_v)::REAL AS battery_v
    FROM ups_samples
    WHERE ts > (SELECT since FROM params)
    GROUP BY b
),
w AS (
    SELECT date_bin((SELECT bsize FROM params), ts, TIMESTAMPTZ 'epoch') AS b,
           AVG(temp_c)::REAL AS temp_c,
           AVG(cloud_cover_pct)::REAL AS cloud_pct,
           AVG(shortwave_wm2)::REAL AS radiation
    FROM weather_samples
    WHERE ts > (SELECT since FROM params)
    GROUP BY b
)
SELECT
    axis.ts AS ts,
    s40.v AS s40_v, s40.ma AS s40_ma,
    s41.v AS s41_v, s41.ma AS s41_ma,
    u.input_v AS ups_in_v, u.battery_v AS ups_batt_v,
    w.temp_c AS w_temp, w.cloud_pct AS w_cloud, w.radiation AS w_rad
FROM axis
LEFT JOIN s40 ON s40.b = axis.ts
LEFT JOIN s41 ON s41.b = axis.ts
LEFT JOIN u   ON u.b   = axis.ts
LEFT JOIN w   ON w.b   = axis.ts
ORDER BY axis.ts
"#
    );

    let rows = sqlx::query(&sql).fetch_all(db.pool()).await?;

    let n = rows.len();
    let mut ts = Vec::with_capacity(n);
    let mut sensor_grid_v = Vec::with_capacity(n);
    let mut sensor_grid_ma = Vec::with_capacity(n);
    let mut sensor_solar_v = Vec::with_capacity(n);
    let mut sensor_solar_ma = Vec::with_capacity(n);
    let mut ups_input_v = Vec::with_capacity(n);
    let mut ups_battery_v = Vec::with_capacity(n);
    let mut weather_temp_c = Vec::with_capacity(n);
    let mut weather_cloud_pct = Vec::with_capacity(n);
    let mut weather_radiation = Vec::with_capacity(n);

    for row in rows {
        let dt: DateTime<Utc> = row.try_get("ts")?;
        ts.push(dt.timestamp());
        sensor_grid_v.push(row.try_get("s40_v")?);
        sensor_grid_ma.push(row.try_get("s40_ma")?);
        sensor_solar_v.push(row.try_get("s41_v")?);
        sensor_solar_ma.push(row.try_get("s41_ma")?);
        ups_input_v.push(row.try_get("ups_in_v")?);
        ups_battery_v.push(row.try_get("ups_batt_v")?);
        weather_temp_c.push(row.try_get("w_temp")?);
        weather_cloud_pct.push(row.try_get("w_cloud")?);
        weather_radiation.push(row.try_get("w_rad")?);
    }

    Ok(HistoryPayload {
        range: match range {
            Range::Hour => "hour",
            Range::Day => "day",
            Range::Week => "week",
            Range::Month => "month",
        },
        bucket,
        ts,
        sensor_grid_v,
        sensor_grid_ma,
        sensor_solar_v,
        sensor_solar_ma,
        ups_input_v,
        ups_battery_v,
        weather_temp_c,
        weather_cloud_pct,
        weather_radiation,
    })
}
