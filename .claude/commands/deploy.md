Run `python deploy/deploy.py` from the project root to deploy the solar controller to the Raspberry Pi.

Steps performed by the script:
1. `npm run build` in frontend/
2. `cargo build --release --target aarch64-unknown-linux-gnu` via WSL (cross-compile ARM64)
3. Upload binary + frontend/dist via SFTP to romain@10.0.0.103:/home/romain/solar-controller
4. Install/restart the systemd service solar-controller

Prerequisites (first time only):
- WSL Ubuntu with `gcc-aarch64-linux-gnu` and `rustup` installed (already done)
- `rustup target add aarch64-unknown-linux-gnu` in WSL (already done)
- `npm install` in frontend/ if not done yet
- On the Pi: `sudo apt install -y nut-client nut-server` and configure NUT in standalone mode (UPS named `ups`, listening on 127.0.0.1:3493). The backend shells out to `/usr/bin/upsc ups@localhost`.

If the build or deploy fails, report the error output and suggest a fix.
After success, confirm the service is running and print the URL: http://10.0.0.103:3000
