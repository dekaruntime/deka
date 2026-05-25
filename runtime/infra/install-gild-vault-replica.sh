#!/usr/bin/env bash
set -euo pipefail

install -d -m 0750 -o gild-vault-replica -g gild /var/lib/gild-vault
install -d -m 0750 -o root -g gild /etc/gild

if [[ ! -f /etc/gild/vault-replication-token ]]; then
  cat >&2 <<'MSG'
gild-vault replication token is missing.

Copy /etc/gild/vault-replication-token from the authoritative node during
replica provisioning, then run this installer again.
MSG
  exit 1
fi

chown gild-vault-replica /etc/gild/vault-replication-token
chmod 0400 /etc/gild/vault-replication-token
