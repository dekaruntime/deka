#!/usr/bin/env bash
set -euo pipefail

dry_run=0
if [[ "${1:-}" == "--dry-run" ]]; then
  dry_run=1
  shift
fi

if [[ "$#" -ne 0 ]]; then
  echo "usage: gild-agent-migrate-groups.sh [--dry-run]" >&2
  exit 64
fi

run() {
  if [[ "$dry_run" -eq 1 ]]; then
    printf '+'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

has_group() {
  local user="$1"
  local group="$2"
  id -nG "$user" | tr ' ' '\n' | awk -v group="$group" '$0 == group { found = 1 } END { exit found ? 0 : 1 }'
}

if ! getent group gild-agents >/dev/null; then
  run groupadd --system gild-agents
  echo "[migrate] created group gild-agents"
fi

for user in $(getent passwd | awk -F: '$1 ~ /^agent-/ { print $1 }'); do
  primary=$(id -gn "$user")
  if [[ "$primary" == "gild-orchestrator" ]]; then
    run usermod -g gild-agents "$user"
    echo "[migrate] $user: primary group gild-orchestrator -> gild-agents"
  fi

  if has_group "$user" gild-orchestrator; then
    run gpasswd -d "$user" gild-orchestrator 2>/dev/null || true
    echo "[migrate] $user: removed supplementary gild-orchestrator"
  fi

  if ! has_group "$user" gild-agents; then
    run gpasswd -a "$user" gild-agents
    echo "[migrate] $user: added supplementary gild-agents"
  fi
done
