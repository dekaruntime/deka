#!/bin/bash
# Production startup for linkha.sh (host-level, not Docker)
# Sources env from .env.local then starts the deka PHPX server on port 4004.
# Port 4004 matches the cloudflared ingress for linkha.sh.

cd /Users/sami/Projects/deka/linkhash/phpx

# Load repo-local env (Postgres, Bluesky OAuth, R2 artifact backend, etc.)
if [ -f .env.local ]; then
  set -a
  source .env.local
  set +a
fi

# Load shared tana env for any shared secrets (kept for pattern consistency).
if [ -f /Users/sami/Projects/tana/.env ]; then
  set -a
  source /Users/sami/Projects/tana/.env
  set +a
fi

export NODE_ENV=production
export PORT=4004

# Purge stale PHPX compile cache so audits don't pick up old artifacts.
rm -rf .cache/phpx_js

exec /Users/sami/Projects/deka/runtime/target/release/cli serve --port 4004 main.phpx
