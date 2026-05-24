#!/usr/bin/env bash
set -euo pipefail

# Install files for gild-vault-proxy on demon.
# This script intentionally does not enable or start the service.

if [[ "$(id -u)" != "0" ]]; then
  echo "Run as root on demon." >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! getent group gild >/dev/null; then
  groupadd --system gild
fi

if ! id gild-vault-proxy >/dev/null 2>&1; then
  useradd \
    --system \
    --no-create-home \
    --home-dir /nonexistent \
    --shell /usr/sbin/nologin \
    --gid nogroup \
    --groups gild \
    gild-vault-proxy
fi

install -m 0755 "${repo_root}/runtime/target/release/gild-vault-proxy" /usr/local/bin/gild-vault-proxy
install -d -m 0750 -o root -g gild /etc/gild

if [[ ! -f /etc/gild/vault-proxy-token ]]; then
  umask 0077
  openssl rand -hex 32 > /etc/gild/vault-proxy-token
  chown root:gild /etc/gild/vault-proxy-token
  chmod 0640 /etc/gild/vault-proxy-token
fi

install -m 0644 "${repo_root}/infra/gg.tana.gild-vault-proxy.service" \
  /etc/systemd/system/gg.tana.gild-vault-proxy.service
systemctl daemon-reload

cat <<'EOF'
Installed gild-vault-proxy files.

Next manual steps:
  systemctl enable gg.tana.gild-vault-proxy.service
  systemctl start gg.tana.gild-vault-proxy.service
  systemctl status gg.tana.gild-vault-proxy.service

Share /etc/gild/vault-proxy-token with tana-admin as VAULT_PROXY_TOKEN.
EOF
