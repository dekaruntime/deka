#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEKA_BIN="${DEKA_BIN:-$ROOT/target/release/cli}"
LINKHASH_ROOT="${LINKHASH_ROOT:-/Users/sami/Projects/deka/linkhash}"
GILD_VCS_DIR="${GILD_VCS_DIR:-$LINKHASH_ROOT/rust/gild-vcs}"
REGISTRY_URL="${REGISTRY_URL:-http://127.0.0.1:8608}"
REGISTRY_TOKEN="${REGISTRY_TOKEN:-test-token}"
REGISTRY_OWNER="${REGISTRY_OWNER:-linkhash-admin}"
PKG_VERSION="${PKG_VERSION:-0.1.0}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/deka-linkhash-e2e.XXXXXX")"
RUN_ID="$(date +%s)"
REGISTRY_REPO="${REGISTRY_REPO:-mvp2e2emodule-${RUN_ID}}"
LOG="$WORK/gild-vcs.log"
PKG_NAME="@${REGISTRY_OWNER}/${REGISTRY_REPO}"

cleanup() {
  if [[ -n "${PID:-}" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$WORK"
}
trap cleanup EXIT

if [[ ! -x "$DEKA_BIN" ]]; then
  echo "[e2e] missing deka cli binary: $DEKA_BIN"
  exit 1
fi

if [[ ! -d "$GILD_VCS_DIR" ]]; then
  echo "[e2e] missing gild-vcs dir: $GILD_VCS_DIR"
  exit 1
fi

(
  cd "$LINKHASH_ROOT/phpx"
  "$DEKA_BIN" db migrate >/dev/null
)

(
  cd "$GILD_VCS_DIR"
  cargo run >"$LOG" 2>&1
) &
PID=$!

for _ in {1..40}; do
  if curl -fsS "$REGISTRY_URL/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done

if ! curl -fsS "$REGISTRY_URL/health" >/dev/null 2>&1; then
  echo "[e2e] registry failed to start"
  cat "$LOG"
  exit 1
fi

if ! curl -fsS -H "Authorization: Bearer $REGISTRY_TOKEN" "$REGISTRY_URL/api/auth/me" >/dev/null 2>&1; then
  echo "[e2e] auth check failed for registry token"
  cat "$LOG"
  exit 1
fi

PKG_DIR="$WORK/publisher"
CONSUMER_DIR="$WORK/consumer"
REMOTE_DIR="$GILD_VCS_DIR/repos/$REGISTRY_OWNER/$REGISTRY_REPO.git"
mkdir -p "$PKG_DIR" "$(dirname "$REMOTE_DIR")"

cat > "$PKG_DIR/deka.json" <<JSON
{
  "name": "$PKG_NAME",
  "version": "$PKG_VERSION",
  "security": {
    "allow": {
      "read": ["./"],
      "write": [],
      "run": [],
      "env": [],
      "net": []
    }
  }
}
JSON

cat > "$PKG_DIR/index.phpx" <<'PHPX'
export function hello_registry($name = 'world') {
  return 'hello ' . $name;
}
PHPX

(
  cd "$PKG_DIR"
  git init >/dev/null
  git config user.email "e2e@example.com"
  git config user.name "e2e"
  git add deka.json index.phpx
  git commit -m "init module" >/dev/null
  git tag "v$PKG_VERSION"
)

rm -rf "$REMOTE_DIR"
git init --bare "$REMOTE_DIR" >/dev/null
(
  cd "$PKG_DIR"
  git remote add origin "$REMOTE_DIR"
  git push origin HEAD >/dev/null
  git push origin "v$PKG_VERSION" >/dev/null
)

(
  cd "$PKG_DIR"
  PUBLISH_OUT="$("$DEKA_BIN" pkg publish \
    --name "$PKG_NAME" \
    --repo "$REGISTRY_REPO" \
    --pkg-version "$PKG_VERSION" \
    --git-ref "v$PKG_VERSION" \
    --token "$REGISTRY_TOKEN" \
    --registry-url "$REGISTRY_URL" \
    --yes 2>&1 || true)"
  if [[ "$PUBLISH_OUT" == *"[publish]"*failed* ]] || [[ "$PUBLISH_OUT" == *"[publish]"*blocked* ]]; then
    echo "[e2e] publish failed"
    echo "$PUBLISH_OUT"
    exit 1
  fi
)

# Create a minimal consumer project without `deka init` so this e2e
# validates registry-backed package installation only.
mkdir -p "$CONSUMER_DIR/app"
cat > "$CONSUMER_DIR/deka.json" <<JSON
{
  "name": "linkhash-e2e-consumer",
  "security": {
    "allow": {
      "read": ["./"],
      "write": ["./deka.lock", "./php_modules"],
      "run": [],
      "env": [],
      "net": []
    }
  }
}
JSON
cat > "$CONSUMER_DIR/deka.lock" <<JSON
{
  "php": {
    "cache": {
      "version": 1,
      "compiler": "phpx-cache-v3",
      "modules": {}
    }
  }
}
JSON
(
  cd "$CONSUMER_DIR"
  INSTALL_OUT="$(LINKHASH_REGISTRY_URL="$REGISTRY_URL" "$DEKA_BIN" pkg install --ecosystem php --spec "$PKG_NAME@$PKG_VERSION" --yes 2>&1 || true)"
  if [[ "$INSTALL_OUT" == *"[install]"*failed* ]]; then
    echo "[e2e] install failed"
    echo "$INSTALL_OUT"
    exit 1
  fi
)

if [[ ! -f "$CONSUMER_DIR/php_modules/$PKG_NAME/index.phpx" ]]; then
  echo "[e2e] installed package missing from php_modules/$PKG_NAME"
  exit 1
fi

cat > "$CONSUMER_DIR/app/main.phpx" <<PHPX
import { hello_registry } from '$PKG_NAME';
echo hello_registry('registry');
PHPX

RUN_OUT="$("$DEKA_BIN" run "$CONSUMER_DIR/app/main.phpx" 2>/dev/null || true)"
if [[ "$RUN_OUT" != *'hello registry'* ]]; then
  echo "[e2e] runtime import smoke failed"
  echo "$RUN_OUT"
  exit 1
fi

echo "[e2e] linkhash publish/install/runtime smoke passed"
