#!/usr/bin/env python3
"""Deploy solar-controller to romain@10.0.0.103"""

import io
import os
import subprocess
import sys
from pathlib import Path

if hasattr(sys.stdout, "buffer"):
    sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8")
    sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8")

import paramiko

HOST = "10.0.0.103"
USER = "romain"
REMOTE_DIR = "/home/romain/solar-controller"
BINARY_LOCAL = "target/aarch64-unknown-linux-musl/release/solar-controller"
FRONTEND_DIST = "frontend/dist"

PROJECT_ROOT = Path(__file__).parent.parent


def run(cmd, cwd=None):
    print(f"\n→ {cmd}")
    result = subprocess.run(cmd, shell=True, cwd=cwd or PROJECT_ROOT)
    if result.returncode != 0:
        print(f"  ERREUR (code {result.returncode})", file=sys.stderr)
        sys.exit(result.returncode)


def ssh_exec(client, cmd):
    print(f"  [pi] {cmd}")
    _, stdout, stderr = client.exec_command(cmd)
    out = stdout.read().decode().strip()
    err = stderr.read().decode().strip()
    if out:
        print(f"       {out}")
    if err:
        print(f"       ERR: {err}", file=sys.stderr)
    return out


def sftp_put_dir(sftp, local_dir: Path, remote_dir: str):
    """Upload a local directory tree via SFTP."""
    for item in local_dir.rglob("*"):
        if item.is_file():
            rel = item.relative_to(local_dir)
            remote_path = f"{remote_dir}/{rel.as_posix()}"
            remote_parent = remote_path.rsplit("/", 1)[0]
            try:
                sftp.stat(remote_parent)
            except FileNotFoundError:
                # mkdir -p equivalent
                parts = remote_parent.replace(REMOTE_DIR, "").strip("/").split("/")
                current = REMOTE_DIR
                for part in parts:
                    current = f"{current}/{part}"
                    try:
                        sftp.stat(current)
                    except FileNotFoundError:
                        sftp.mkdir(current)
            print(f"  upload {rel}")
            sftp.put(str(item), remote_path)


def main():
    print("=" * 50)
    print("  Solar Controller — Déploiement")
    print("=" * 50)

    # 1. Build frontend
    run("npm run build", cwd=PROJECT_ROOT / "frontend")

    # 2. Cross-compile backend via WSL (gcc-aarch64-linux-gnu + cargo dans WSL)
    wsl_path = PROJECT_ROOT.as_posix().replace("s:/", "/mnt/s/").replace("S:/", "/mnt/s/")
    run(
        f'wsl bash -c "source ~/.cargo/env && cd {wsl_path}/backend && '
        f'cargo build --release --target aarch64-unknown-linux-musl"'
    )

    binary = PROJECT_ROOT / BINARY_LOCAL
    if not binary.exists():
        print(f"Binaire introuvable : {binary}", file=sys.stderr)
        sys.exit(1)

    # 3. Connect SSH (clé publique installée dans ~/.ssh/authorized_keys côté Pi).
    # `look_for_keys=True` cherche ~/.ssh/id_rsa, id_ecdsa, id_ed25519 ; `allow_agent=True`
    # utilise un agent SSH si présent. Pas de password = pas de secret en clair dans le repo.
    print("\n→ Connexion SSH à la Pi…")
    client = paramiko.SSHClient()
    client.load_system_host_keys()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, username=USER, look_for_keys=True, allow_agent=True)
    sftp = client.open_sftp()

    # 4. Créer l'arborescence distante
    for d in [REMOTE_DIR, f"{REMOTE_DIR}/frontend", f"{REMOTE_DIR}/frontend/dist"]:
        try:
            sftp.stat(d)
        except FileNotFoundError:
            sftp.mkdir(d)

    # 5. Stopper le service (sinon le binaire est verrouillé)
    print("\n→ Stop service…")
    ssh_exec(client, "sudo systemctl stop solar-controller 2>/dev/null || true")

    # 6. Upload binaire
    print(f"\n→ Upload binaire ({binary.stat().st_size // 1024} KB)…")
    sftp.put(str(binary), f"{REMOTE_DIR}/solar-controller")
    ssh_exec(client, f"chmod +x {REMOTE_DIR}/solar-controller")

    # 6. Upload frontend/dist
    print("\n→ Upload frontend/dist…")
    dist_path = PROJECT_ROOT / FRONTEND_DIST
    sftp_put_dir(sftp, dist_path, f"{REMOTE_DIR}/frontend/dist")

    # 7. Installer service systemd (idempotent)
    print("\n→ Installation service systemd…")
    service_local = PROJECT_ROOT / "deploy" / "solar-controller.service"
    sftp.put(str(service_local), "/tmp/solar-controller.service")
    ssh_exec(client, "sudo mv /tmp/solar-controller.service /etc/systemd/system/")
    ssh_exec(client, "sudo systemctl daemon-reload")
    ssh_exec(client, "sudo systemctl enable solar-controller")
    ssh_exec(client, "sudo systemctl restart solar-controller")

    # 8. Vérification
    print("\n→ Statut du service :")
    ssh_exec(client, "sudo systemctl status solar-controller --no-pager -l")

    sftp.close()
    client.close()

    print(f"\n✓ Déployé → http://{HOST}:3000")


if __name__ == "__main__":
    main()
