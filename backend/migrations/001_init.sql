-- Schéma de stockage des relevés solar-controller.
-- Idempotent : exécuté à chaque boot du service via db::run_migrations.

CREATE TABLE IF NOT EXISTS sensor_samples (
    ts          TIMESTAMPTZ NOT NULL,
    address     SMALLINT    NOT NULL,
    bus_v       REAL        NOT NULL,
    current_ma  REAL        NOT NULL,
    PRIMARY KEY (ts, address)
);
CREATE INDEX IF NOT EXISTS ix_sensor_ts ON sensor_samples (ts DESC);

CREATE TABLE IF NOT EXISTS ups_samples (
    ts            TIMESTAMPTZ PRIMARY KEY,
    input_v       REAL,
    input_hz      REAL,
    output_v      REAL,
    load_pct      REAL,
    battery_pct   REAL,
    battery_v     REAL,
    runtime_s     INTEGER,
    status        TEXT
);
CREATE INDEX IF NOT EXISTS ix_ups_ts ON ups_samples (ts DESC);

CREATE TABLE IF NOT EXISTS weather_samples (
    ts                TIMESTAMPTZ PRIMARY KEY,
    temp_c            REAL,
    cloud_cover_pct   REAL,
    shortwave_wm2     REAL,
    source            TEXT
);
CREATE INDEX IF NOT EXISTS ix_weather_ts ON weather_samples (ts DESC);

CREATE TABLE IF NOT EXISTS relay_events (
    ts    TIMESTAMPTZ PRIMARY KEY,
    state TEXT NOT NULL
);
ALTER TABLE relay_events ADD COLUMN IF NOT EXISTS reason TEXT;

-- Persistance des paramètres modifiables à chaud (auto-switch on/off, etc.).
-- Utilisé via db::get_setting_bool / set_setting_bool.
CREATE TABLE IF NOT EXISTS settings (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Prévisions journalières (lever/coucher + somme rayonnement) pour la
-- décision auto fin-de-journée. Stocke 2-3 jours glissants.
-- shortwave_sum_kwh : somme du rayonnement journalier en kWh/m²
-- (Open-Meteo renvoie shortwave_radiation_sum en MJ/m² → divisé par 3.6).
CREATE TABLE IF NOT EXISTS forecast_daily (
    date              DATE PRIMARY KEY,
    sunrise           TIMESTAMPTZ,
    sunset            TIMESTAMPTZ,
    shortwave_sum_kwh REAL,
    fetched_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS ix_forecast_daily_date ON forecast_daily (date DESC);
