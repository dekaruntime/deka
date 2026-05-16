#!/bin/bash
# Publish all PHPX stdlib modules to linkhash as @deka/* packages.
# Also publishes @tana/store.
#
# Requires a running linkhash server at http://localhost:9418 and valid tokens:
#   DEKA_TOKEN — owner=deka, scopes include packages:write,repo:write
#   TANA_TOKEN — owner=tana, scopes include packages:write,repo:write
set -euo pipefail

REGISTRY="${REGISTRY:-http://localhost:9418}"
DEKA_TOKEN="${DEKA_TOKEN:?need DEKA_TOKEN env}"
TANA_TOKEN="${TANA_TOKEN:?need TANA_TOKEN env}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STDLIB_DIR="${STDLIB_DIR:-$SCRIPT_DIR/php_modules}"
TANA_STORE_DIR="${TANA_STORE_DIR:-$HOME/Projects/tana/store/default/php_modules/@tana/store}"
WORK_ROOT="$(mktemp -d -t linkhash-publish.XXXXXX)"

VERSION="0.1.0"
PUBLISH_PACKAGES="${PUBLISH_PACKAGES:-}"

# Modules under $STDLIB_DIR that have ONE flat directory of .phpx files.
# Skip: @user (user-space), _test, _tmp, encoding (has subdirs), stdlib.json, deka.php
# We publish encoding/* as @deka/encoding-json, @deka/encoding-binary separately.
FLAT_MODULES=(
  array
  auth
  buffer
  bytes
  component
  cookies
  core
  crypto
  db
  framework
  fs
  json
  jwt
  neo4j
  redis
  string
  tcp
  time
  tls
  ui
)

NESTED_ENCODING=(
  json
  binary
)

echo "=== Work dir: $WORK_ROOT ==="
failures=0

# ---------- helpers ----------

should_publish() {
  local package="$1"
  if [ -z "$PUBLISH_PACKAGES" ]; then
    return 0
  fi
  case " $PUBLISH_PACKAGES " in
    *" $package "*) return 0 ;;
    *) return 1 ;;
  esac
}

make_deka_json() {
  local name="$1"
  local description="$2"
  local target="$3"

  # If a deka.json already exists, merge (ensuring name/version are correct).
  if [ -f "$target/deka.json" ]; then
    # Update name/version, keep existing fields
    node -e "
      const fs = require('fs');
      const path = '$target/deka.json';
      const j = JSON.parse(fs.readFileSync(path, 'utf8'));
      j.name = '$name';
      j.version = '$VERSION';
      if (!j.description) j.description = '$description';
      if (!j.main) j.main = 'index.phpx';
      if (!j.security) j.security = { allow: { run: true } };
      else if (!j.security.allow) j.security.allow = { run: true };
      else if (j.security.allow.run === undefined) j.security.allow.run = true;
      delete j['deka.security'];
      fs.writeFileSync(path, JSON.stringify(j, null, 2) + '\n');
    " 2>/dev/null || cat > "$target/deka.json" <<EOF
{
  "name": "$name",
  "version": "$VERSION",
  "description": "$description",
  "main": "index.phpx",
  "security": { "allow": { "run": true } }
}
EOF
  else
    cat > "$target/deka.json" <<EOF
{
  "name": "$name",
  "version": "$VERSION",
  "description": "$description",
  "main": "index.phpx",
  "security": { "allow": { "run": true } }
}
EOF
  fi
}

# Ensure a repo exists on the git server under $owner/$repo.
# Idempotent (409 on exists is acceptable).
ensure_repo() {
  local owner="$1"
  local repo="$2"
  local token="$3"

  local http_code
  http_code=$(curl -s -o /tmp/repo-create.json -w "%{http_code}" \
    -X POST "$REGISTRY/api/repos/$repo" \
    -H "Authorization: Bearer $token" \
    -H "Content-Type: application/json" \
    -d '{}')

  if [ "$http_code" = "201" ] || [ "$http_code" = "409" ] || grep -q "already exists" /tmp/repo-create.json 2>/dev/null; then
    return 0
  fi
  if [ "$http_code" = "500" ] && grep -q "already exists" /tmp/repo-create.json 2>/dev/null; then
    return 0
  fi
  echo "ensure_repo failed ($http_code): $(cat /tmp/repo-create.json)" >&2
  return 1
}

set_visibility_public() {
  local owner="$1"
  local repo="$2"
  local token="$3"

  local http_code
  http_code=$(curl -s -o /tmp/repo-visibility.json -w "%{http_code}" \
    -X PATCH "$REGISTRY/api/repos/$owner/$repo/visibility" \
    -H "Authorization: Bearer $token" \
    -H "Content-Type: application/json" \
    -d '{"visibility":"public"}')

  if [ "$http_code" = "200" ]; then
    return 0
  fi
  echo "set_visibility_public warning ($http_code): $(cat /tmp/repo-visibility.json)" >&2
  return 1
}

# Clone source files + deka.json into a working repo, git commit, tag, push.
# Args:
#   $1 — owner (deka | tana)
#   $2 — repo name (e.g. crypto, store, encoding-json)
#   $3 — package name (e.g. @deka/crypto)
#   $4 — source directory with the module's files (no .git)
#   $5 — token
publish_one() {
  local owner="$1"
  local repo="$2"
  local name="$3"
  local src_dir="$4"
  local token="$5"
  local description="$6"

  echo ""
  echo "=== $name ($owner/$repo) ==="
  echo "  src: $src_dir"

  ensure_repo "$owner" "$repo" "$token" || return 1

  local work="$WORK_ROOT/$owner--$repo"
  rm -rf "$work"
  mkdir -p "$work"
  # Copy files (exclude dotfiles/dirs to avoid .git)
  cp -R "$src_dir"/. "$work"/
  # Remove any nested .git
  rm -rf "$work/.git"

  # Generate / fix deka.json
  make_deka_json "$name" "$description" "$work"

  (
    cd "$work"
    git init -q -b main
    git config user.email "registry@tana.gg"
    git config user.name "linkhash-registry"
    git config commit.gpgsign false
    git config tag.gpgsign false
    git add -A
    git commit -q -m "$name@$VERSION"
    git tag "v$VERSION" || git -c tag.gpgsign=false tag "v$VERSION"
    git remote add origin "$REGISTRY/$owner/$repo"
    git -c http.extraHeader="Authorization: Bearer $token" push -q origin main --tags --force
  )

  # Preflight
  local preflight
  preflight=$(curl -s -X POST "$REGISTRY/api/packages/preflight" \
    -H "Authorization: Bearer $token" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"$name\",\"version\":\"$VERSION\",\"repo\":\"$repo\",\"git_ref\":\"v$VERSION\"}")
  if echo "$preflight" | grep -q '"error"'; then
    echo "  preflight error: $preflight" >&2
    return 1
  fi

  # Publish
  local publish
  publish=$(curl -s -X POST "$REGISTRY/api/packages/publish" \
    -H "Authorization: Bearer $token" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"$name\",\"version\":\"$VERSION\",\"repo\":\"$repo\",\"git_ref\":\"v$VERSION\",\"description\":\"$description\"}")
  if echo "$publish" | grep -q '"error"'; then
    echo "  publish error: $publish" >&2
    return 1
  fi
  echo "  published: $(echo "$publish" | head -c 200)"

  # @deka/* and @tana/store: leave default (private) for @tana, flip to public for @deka
  if [ "$owner" = "deka" ]; then
    if set_visibility_public "$owner" "$repo" "$token"; then
      echo "  visibility: public"
    else
      echo "  visibility: unchanged"
    fi
  fi
}

# ---------- main ----------

# Flat modules under deka scope
for module in "${FLAT_MODULES[@]}"; do
  package="@deka/$module"
  if ! should_publish "$package"; then
    continue
  fi
  src="$STDLIB_DIR/$module"
  if [ ! -d "$src" ]; then
    echo "SKIP: $module (dir not found)" >&2
    continue
  fi
  publish_one "deka" "$module" "@deka/$module" "$src" "$DEKA_TOKEN" "PHPX stdlib: $module" || {
    echo "FAILED: @deka/$module" >&2
    failures=$((failures + 1))
  }
done

# Encoding subdirs: encoding/json -> @deka/encoding-json, encoding/binary -> @deka/encoding-binary
for sub in "${NESTED_ENCODING[@]}"; do
  package="@deka/encoding-$sub"
  if ! should_publish "$package"; then
    continue
  fi
  src="$STDLIB_DIR/encoding/$sub"
  if [ ! -d "$src" ]; then continue; fi
  publish_one "deka" "encoding-$sub" "@deka/encoding-$sub" "$src" "$DEKA_TOKEN" "PHPX stdlib: encoding/$sub" || {
    echo "FAILED: @deka/encoding-$sub" >&2
    failures=$((failures + 1))
  }
done

# @tana/store
if should_publish "@tana/store"; then
  publish_one "tana" "store" "@tana/store" "$TANA_STORE_DIR" "$TANA_TOKEN" "Tana storefront module" || {
    echo "FAILED: @tana/store" >&2
    failures=$((failures + 1))
  }
fi

echo ""
echo "=== Done. Work dir: $WORK_ROOT ==="
if [ "$failures" -gt 0 ]; then
  echo "=== Failed publishes: $failures ===" >&2
  exit 1
fi
