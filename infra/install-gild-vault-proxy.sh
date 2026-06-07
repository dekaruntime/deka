#!/usr/bin/env bash
set -euo pipefail

# Install files for gild-vault-proxy on demon.
# This script intentionally does not enable or start the service.

dry_run="${DRY_RUN:-0}"
with_mtls=0
for arg in "$@"; do
  case "${arg}" in
    --with-mtls)
      with_mtls=1
      ;;
    -h|--help)
      cat <<'EOF'
Usage: install-gild-vault-proxy.sh [--with-mtls]

Default install keeps the existing bearer-only listener on 0.0.0.0:9444.
--with-mtls initializes /etc/gild/vault-proxy-ca.{pem,key}, generates the
proxy server certificate, and installs a drop-in that serves mTLS on :9445.
Operators then issue per-machine client certs with:
  gild vault ca issue <client-cn>
EOF
      exit 0
      ;;
    *)
      echo "unknown argument: ${arg}" >&2
      exit 1
      ;;
  esac
done

if [[ "${dry_run}" == "1" ]]; then
  cat <<'EOF'
DRY_RUN=1 gild-vault-proxy install plan:
  - ensure system group: gild
  - ensure system user: gild-vault-proxy
  - add supplementary group: gild
  - install binary: /usr/local/bin/gild-vault-proxy
  - install token file: /etc/gild/vault-proxy-token (root:gild 0640)
  - install systemd unit: /etc/systemd/system/gg.tana.gild-vault-proxy.service
  - proxy vault socket: VAULT_SOCKET=/run/gild-vault/sock
  - expected vault socket mode/group: 0660 gild
  - optional mTLS: pass --with-mtls to initialize CA and :9445 TLS listener
EOF
  exit 0
fi

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

if [[ "${with_mtls}" == "1" ]]; then
  /usr/local/bin/gild vault ca init
  server_bundle="$(mktemp)"
  /usr/local/bin/gild vault ca issue --server gild-vault-proxy-demon > "${server_bundle}"
  awk '
    /BEGIN CERTIFICATE/ { in_cert=1 }
    in_cert { print }
    /END CERTIFICATE/ { in_cert=0 }
  ' "${server_bundle}" > /etc/gild/vault-proxy.pem
  awk '
    /BEGIN PRIVATE KEY/ { in_key=1 }
    in_key { print }
    /END PRIVATE KEY/ { in_key=0 }
  ' "${server_bundle}" > /etc/gild/vault-proxy.key
  rm -f "${server_bundle}"
  chown root:gild /etc/gild/vault-proxy-ca.pem /etc/gild/vault-proxy.pem
  chown root:root /etc/gild/vault-proxy-ca.key /etc/gild/vault-proxy.key
  chmod 0440 /etc/gild/vault-proxy-ca.pem /etc/gild/vault-proxy.pem
  chmod 0400 /etc/gild/vault-proxy-ca.key /etc/gild/vault-proxy.key
  install -d -m 0755 /etc/systemd/system/gg.tana.gild-vault-proxy.service.d
  cat >/etc/systemd/system/gg.tana.gild-vault-proxy.service.d/10-mtls.conf <<'EOF'
[Service]
ExecStart=
ExecStart=/usr/local/bin/gild-vault-proxy --tls-bind 0.0.0.0:9445 --tls-ca /etc/gild/vault-proxy-ca.pem --tls-cert /etc/gild/vault-proxy.pem --tls-key /etc/gild/vault-proxy.key
EOF
fi
systemctl daemon-reload

cat <<'EOF'
Installed gild-vault-proxy files.

Next manual steps:
  systemctl enable gg.tana.gild-vault-proxy.service
  systemctl start gg.tana.gild-vault-proxy.service
  systemctl status gg.tana.gild-vault-proxy.service

Share /etc/gild/vault-proxy-token with tana-admin as VAULT_PROXY_TOKEN.

Upgrade path for mTLS:
  1. Re-run this installer with --with-mtls.
  2. Restart gg.tana.gild-vault-proxy.service.
  3. Issue one client identity per caller: gild vault ca issue <machine-cn>.
  4. Keep the bearer token configured; mTLS is an additional identity layer.
EOF
