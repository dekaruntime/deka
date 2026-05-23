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

if ! getent group gild-agents >/dev/null; then
  run groupadd --system gild-agents
fi

getent passwd \
  | awk -F: '$1 ~ /^agent-/ { print $1 }' \
  | while IFS= read -r user; do
      [[ -n "$user" ]] || continue
      if id -nG "$user" | tr ' ' '\n' | grep -qx gild-orchestrator; then
        run gpasswd -d "$user" gild-orchestrator
      fi
      if ! id -nG "$user" | tr ' ' '\n' | grep -qx gild-agents; then
        run gpasswd -a "$user" gild-agents
      fi
    done
