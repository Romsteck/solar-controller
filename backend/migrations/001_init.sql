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
