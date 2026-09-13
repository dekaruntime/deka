#!/usr/bin/env bash
# rfd#61 phase 2: cli crate is composition only.
set -euo pipefail
cd "$(dirname "$0")/.."

fail=0

allowed_src=$(mktemp)
trap 'rm -f "$allowed_src"' EXIT
cat > "$allowed_src" <<'EOF'
crates/cli/src/main.rs
crates/cli/src/lib.rs
crates/cli/src/context.rs
crates/cli/src/cli/mod.rs
EOF

while IFS= read -r file; do
    if ! grep -qx "$file" "$allowed_src"; then
        echo "FAIL: $file is not part of the sanctioned cli composition surface."
        fail=1
    fi
done < <(find crates/cli/src -name '*.rs' -type f | sort)

# Handlers live in owner crates. CommandSpec in cli would mean a handler leaked back.
if grep -nE "CommandSpec \{|add_command\(|handler: cmd" crates/cli/src -r --include='*.rs' >/dev/null; then
    echo "FAIL: crates/cli/src defines a command implementation (belongs in an owner crate)."
    grep -nE "CommandSpec \{|add_command\(|handler: cmd" crates/cli/src -r --include='*.rs' || true
    fail=1
fi

budget=800
lines=$(find crates/cli/src -name '*.rs' -type f -print0 | xargs -0 cat | wc -l | tr -d ' ')
if [ "$lines" -gt "$budget" ]; then
    echo "FAIL: crates/cli/src is $lines lines (budget $budget). Move implementations to owner crates."
    fail=1
fi

allowlist=scripts/cli-dep-allowlist.txt
# Direct [dependencies] package names (not workspace tables, not features).
deps=$(awk '
    $0 == "[dependencies]" {in_deps=1; next}
    $0 ~ /^\[/ {in_deps=0}
    in_deps && $0 ~ /^[a-zA-Z0-9_-]+/ {
        split($1, a, " ")
        name=a[1]
        print name
    }
' crates/cli/Cargo.toml)
while IFS= read -r dep; do
    [ -z "$dep" ] && continue
    if ! grep -qx "$dep" "$allowlist"; then
        echo "FAIL: crates/cli depends on '$dep' which is not on $allowlist"
        fail=1
    fi
done <<< "$deps"

if [ "$fail" -ne 0 ]; then
    echo
    echo "See rfd#61: cli is registry composition only."
    exit 1
fi
echo "cli surface gate: ok ($lines lines / budget $budget)"
