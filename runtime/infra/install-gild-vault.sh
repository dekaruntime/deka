#!/usr/bin/env bash
set -euo pipefail

install -d -m 0750 -o gild-vault -g gild /var/lib/gild-vault
install -d -m 0750 -o root -g gild /etc/gild

if [[ ! -f /etc/gild/vault-replication-token ]]; then
  gild vault init-replication-token
fi

if [[ ! -f /etc/gild/vault-master.key ]]; then
  cat >&2 <<'MSG'
gild-vault master key is missing.

Run:
  gild vault init

Then migrate the current tmpfs plaintext state before restarting:
  gild vault migrate-from-plaintext
MSG
fi
