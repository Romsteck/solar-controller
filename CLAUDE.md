# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

A Raspberry Pi controller that switches a load between **grid power** and **solar power** by driving two GPIO-controlled relays, exposes the live state over an HTTP API, and serves a small React UI. Runs on `piswitch` (`romain@10.0.0.103`, aarch64 Raspbian Bookworm) under systemd.

The whole point of the backend is **safety around the relay pair**: the two relays must never be commanded closed at the same time (= short-circuit between grid and solar). Most of the code in [backend/src/relay.rs](backend/src/relay.rs) exists to enforce that invariant, and any change near it must preserve the break-before-make sequence.

## Commands

Builds and deploys are driven from Windows; the cross-compile happens inside WSL Ubuntu.

- **Cross-compile + frontend build + deploy to Pi**: `python deploy/deploy.py` (or `deploy/deploy.sh`) from project root. This runs `npm run build`, then `cargo build --release --target aarch64-unknown-linux-musl` inside WSL, then SFTPs the binary + `frontend/dist` to the Pi, installs `deploy/solar-controller.service`, and restarts systemd. Requires SSH key auth to `romain@10.0.0.103`.
- **Backend type-check (no Pi needed)**: from WSL, `cargo check --target aarch64-unknown-linux-gnu` in the workspace root. Native `cargo check` on Windows fails because `rppal` is Linux-only.
- **Backend unit tests**: `cargo test -p solar-controller` (runs only the pure-logic tests in `relay.rs`; anything touching real GPIO requires a Pi).
- **Frontend dev server**: `cd frontend && npm run dev`. Vite proxies `/api` to `http://10.0.0.103:3000`, so the dev UI talks to the live Pi.
- **Frontend production build**: `cd frontend && npm run build` (runs `tsc` then `vite build` into `frontend/dist/`).
- **Service inspection on the Pi**: `ssh romain@10.0.0.103 'sudo systemctl status solar-controller --no-pager -l'` and `journalctl -u solar-controller -n 100`.

## Backend architecture (`backend/src/`)

Single Axum binary, three concurrent loops + HTTP server, all sharing `AppState`.

**State split** ([state.rs](backend/src/state.rs)): `AppState` holds two locks on purpose.
- `inner: Arc<parking_lot::Mutex<InnerState>>` — fast, sync, never-poisoning lock for sensor/UPS readings and `published_state` (the relay state surfaced to `/api/status`). Held only briefly.
- `relay: Arc<tokio::sync::Mutex<RelayController>>` — async lock held for the full duration of a switch (which awaits a sleep). The mutex itself is the concurrency guarantee against two simultaneous switches.

**Relay safety machine** ([relay.rs](backend/src/relay.rs)): HAT is **active-LOW** (LOW = closed). `switch_to(target, settle)` always: open both → sleep `max(settle, RELAY_SETTLE_MIN)` → re-verify both pins are HIGH → drive only the target LOW → call `verify()`. `RELAY_SETTLE_MIN = 500ms` is the load-bearing constant; the unit test `relay_settle_min_is_at_least_500ms` exists to break loudly if anyone lowers it. On any inconsistency, `verify()` and `switch_to` both call `open_all()` first, then return the error — there is no path that returns an error while leaving a relay closed.

**Failure-mode layering** for "the process dies":
1. `Drop for RelayController` drives both pins HIGH.
2. `set_reset_on_drop(true)` releases pins to input mode if Drop doesn't run.
3. The HAT's pull-ups then hold the pins HIGH = relays open.
4. `into_output_high()` (not `into_output()`) is used at startup so a previous crashed process leaving a pin LOW in the GPIO register does not re-close a relay on boot.
5. A SIGTERM/SIGINT handler in `main.rs` calls `open_all()` with a 3 s timeout before `process::exit(0)`. systemd `KillSignal=SIGTERM`, `RestartSec=15` is set so coils have time to physically release before a restart.

**Watchdog** ([watchdog.rs](backend/src/watchdog.rs)): every 500 ms, `try_lock` on the relay mutex (so it never blocks an in-flight switch); on success calls `verify()` which re-reads GPIO. If physical state diverges from logical state, it forces `open_all()` and republishes.

**Routes** ([routes.rs](backend/src/routes.rs)):
- `GET /api/status` — uses `try_lock` on the relay mutex purely as a "switch in progress" probe, then reads from `inner`. Never blocks.
- `POST /api/switch` — `try_lock` returns 409 if a switch is already in flight (avoids queueing clicks). Toggles via `current_state().next_target()`. Always republishes `current_state()` after, even on error (`switch_to` left it as `Open`).

**Sensor poll** ([sensors.rs](backend/src/sensors.rs)): every 1 s, reads two I²C devices at `0x40` and `0x41`. **These are INA236 (Joy-it SBC-DVA), not INA219/INA226** despite common labeling — bus voltage is 1.6 mV/LSB (cast `i16 → u16 → f32`), shunt is 8 mΩ on PCB so current is `shunt_raw × 0.3125 mA`. Don't "fix" the formula to a generic INA219 one.

**UPS poll** ([ups.rs](backend/src/ups.rs)): every 2 s, shells out to `/usr/bin/upsc ups@localhost` with a 3 s timeout. Logs only on the first failure (and on recovery) to avoid spamming. **Every field in `UpsReading` is `Option`** because the GreenCell PowerProof firmware never reports `battery.charge` / `battery.runtime` — that is not a parsing bug.

**Static files**: Axum's `ServeDir` serves `frontend/dist/` as the fallback, so `/api/*` are routes and everything else falls through to the SPA.

## Frontend architecture (`frontend/src/`)

React 18 + Vite, intentionally tiny: [App.tsx](frontend/src/App.tsx) polls `/api/status` every 1 s, displays `NetworkBadge`/`SwitchButton`/`UpsCard`/`SensorCard`, and POSTs to `/api/switch`. `relay_state === 'open'` is treated as the "safety" state and surfaces a red banner — the UI is the user-visible signal that something has tripped. No state library, no router. Inline styles throughout.

## Build & cross-compile setup

- Workspace root `Cargo.toml` declares `members = ["backend"]`.
- Release profile is size-optimized: `opt-level = "z"`, `lto = true`, `strip = true`.
- [.cargo/config.toml](.cargo/config.toml) wires both ARM64 targets:
  - `aarch64-unknown-linux-gnu` → `aarch64-linux-gnu-gcc` linker (used for local `cargo check`).
  - `aarch64-unknown-linux-musl` → `rust-lld` with `+crt-static` (this is what `deploy.py` ships, so the binary doesn't depend on the Pi's glibc).
- Deploy script lives outside the Cargo workspace and uses `paramiko` for SFTP/SSH (no shelling out to `scp`/`ssh`).

## Hardware constraints worth knowing before you touch code

- Relay pins: **GPIO 20 = grid, GPIO 26 = solar**. Active-LOW HAT.
- I²C addresses: `0x40` (battery/grid side, ~25 V in nominal Grid mode), `0x41` (solar side, reads ~0 V when Grid is active).
- UPS over USB through NUT in standalone mode (`nut-server` listening on `127.0.0.1:3493`, UPS named `ups`). The systemd unit declares `After=nut-server.service` / `Wants=nut-server.service`.
