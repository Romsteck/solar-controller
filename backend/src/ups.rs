use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::time;
use crate::state::{AppState, UpsReading};

const UPSC_BIN: &str = "/usr/bin/upsc";
const UPS_NAME: &str = "ups@localhost";
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const READ_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn poll_loop(state: AppState) {
    let mut interval = time::interval(POLL_INTERVAL);
    let mut consecutive_errors: u32 = 0;
    loop {
        interval.tick().await;
        match read_ups().await {
            Ok(reading) => {
                if consecutive_errors > 0 {
                    tracing::info!(
                        previous_errors = consecutive_errors,
                        "UPS lecture rétablie"
                    );
                    consecutive_errors = 0;
                }
                state.inner.lock().ups = Some(reading);
            }
            Err(e) => {
                state.inner.lock().ups = None;
                if consecutive_errors == 0 {
                    tracing::warn!(error = %e, "UPS lecture échouée");
                } else {
                    tracing::debug!(error = %e, n = consecutive_errors, "UPS toujours indisponible");
                }
                consecutive_errors = consecutive_errors.saturating_add(1);
            }
        }
    }
}

async fn read_ups() -> anyhow::Result<UpsReading> {
    let fut = Command::new(UPSC_BIN).arg(UPS_NAME).output();
    let out = time::timeout(READ_TIMEOUT, fut)
        .await
        .map_err(|_| anyhow::anyhow!("upsc timeout après {}s", READ_TIMEOUT.as_secs()))??;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("upsc exit {} : {}", out.status, stderr.trim());
    }

    let text = String::from_utf8(out.stdout)?;
    let mut r = UpsReading::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(": ") else { continue };
        match k {
            "input.voltage" => r.input_voltage_v = v.parse().ok(),
            "input.frequency" => r.input_frequency_hz = v.parse().ok(),
            "output.voltage" => r.output_voltage_v = v.parse().ok(),
            "ups.load" => r.load_pct = v.parse().ok(),
            "battery.charge" => r.battery_pct = v.parse().ok(),
            "battery.voltage" => r.battery_voltage_v = v.parse().ok(),
            "battery.runtime" => r.runtime_s = v.parse().ok(),
            "ups.status" => r.status = Some(v.trim().to_string()),
            _ => {}
        }
    }

    r.last_seen = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Ok(r)
}
