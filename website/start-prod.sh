#!/bin/bash
# Production startup for deka.gg website (host-level, not Docker)
# Sources env from .env.local then starts the Next.js server on port 4003.
# Port 4003 matches the cloudflared ingress for deka.gg.
#
# Path is resolved from the script's own location so deploys can run from
# any checkout (sami's working tree, deka-deploy/, etc.) without hardcoded
# paths drifting between machines.

cd "$(dirname "${BASH_SOURCE[0]}")"

# Load repo-local env (any docs-site-specific config)
if [ -f .env.local ]; then
  set -a
  source .env.local
  set +a
fi

# Load shared tana env (mostly for parity with the other apps; deka.gg
# usually doesn't need anything from here, but keep the pattern consistent).
if [ -f /Users/sami/Projects/tana/.env ]; then
  set -a
  source /Users/sami/Projects/tana/.env
  set +a
fi

export NODE_ENV=production
export PORT=4003

# Call next directly so PORT env var is honored.
exec /Users/sami/.bun/bin/bun x next start --port 4003
