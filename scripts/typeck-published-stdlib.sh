#!/usr/bin/env bash
# Typecheck each published @deka/* tarball against this checkout's CLI.
# Catches stdlib that compiled on an old compiler and broke on current main
# (dekaruntime/deka#405).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CLI="${CLI:-$ROOT/target/release/cli}"
if [[ ! -x "$CLI" ]]; then
  CLI="$ROOT/target/debug/cli"
fi
if [[ ! -x "$CLI" ]]; then
  echo "error: no deka CLI at $CLI; build -p cli first" >&2
  exit 1
fi

REGISTRY="${REGISTRY_URL:-https://deka.gg/api/registry}"
# Live index package names. Probe each; skip if the registry 404s.
PACKAGES=(
  auth bytes cookies crypto fs http io json jwt tcp time tls
)

failed=0
workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

for name in "${PACKAGES[@]}"; do
  meta="$(curl -fsS "$REGISTRY/$name.json" || true)"
  if [[ -z "$meta" ]]; then
    echo "skip $name (no registry json)"
    continue
  fi
  echo "=== $name ==="
  proj="$workdir/$name"
  mkdir -p "$proj"
  printf '%s\n' "{\"name\":\"typeck-$name\",\"version\":\"0.0.0\",\"security\":{\"prompt\":false}}" >"$proj/deka.json"
  printf '%s\n' '{"lockfileVersion":1,"packages":{}}' >"$proj/deka.lock"
  if ! (cd "$proj" && DEKA_SECURITY_NO_PROMPT=1 "$CLI" add "$name" --yes --no-prompt); then
    echo "FAIL $name: deka add"
    failed=1
    continue
  fi
  # `deka check` is a SINGLE-FILE typecheck: it does not resolve imports, so
  # checking a package's index.ds directly reports every imported name as
  # unknown and cascades. That is a false failure for any package with
  # dependencies (auth, jwt, http), and a true result only by luck for those
  # without.
  #
  # `deka run` on a consumer walks the module graph and typechecks the package
  # in place. Validated against bytes@0.2.0, whose `unknown type object` this
  # catches while `deka check` on a consumer and `deka build` both miss it.
  pkg_index=""
  for candidate in \
    "$proj/ds_modules/@deka/$name/index.ds" \
    "$proj/ds_modules/$name/index.ds" \
    "$proj/php_modules/@deka/$name/index.ds" \
    "$proj/php_modules/$name/index.ds"
  do
    if [[ -f "$candidate" ]]; then
      pkg_index="$candidate"
      break
    fi
  done
  if [[ -z "$pkg_index" ]]; then
    echo "FAIL $name: no index.ds after install"
    failed=1
    continue
  fi

  # Import one real export so the package enters the graph. A bare named
  # import is enough; nothing is called, so no argument types are needed.
  # || true: grep exits 1 on no match and head can SIGPIPE it; neither is fatal
  # here, an empty symbol is handled below. Without this, set -o pipefail aborts
  # the whole run on the first package that yields nothing.
  symbol="$( { grep -oE '^export (async )?fn [A-Za-z_][A-Za-z0-9_]*' "$pkg_index" || true; } | head -1 | awk '{print $NF}')"
  if [[ -z "$symbol" ]]; then
    echo "skip $name (no exported fn to import)"
    continue
  fi

  if [[ "$name" == "io" ]]; then
    printf 'import { echo } from "io"\necho("ok")\n' >"$proj/consumer.ds"
  else
    (cd "$proj" && DEKA_SECURITY_NO_PROMPT=1 "$CLI" add io --yes --no-prompt >/dev/null 2>&1) || true
    printf 'import { %s } from "%s"\nimport { echo } from "io"\necho("ok")\n' \
      "$symbol" "$name" >"$proj/consumer.ds"
  fi

  out="$(cd "$proj" && DEKA_SECURITY_NO_PROMPT=1 "$CLI" run consumer.ds 2>&1 || true)"
  # Any diagnostic whose location points into the installed package is a real
  # failure. Matching on error text instead missed whole classes: an earlier
  # pattern list passed @deka/fs while it was emitting
  # "`await` expected Promise<T>" on three lines.
  if grep -qE '^[0-9]+:[0-9]+:.*(ds_modules|php_modules)/' <<<"$out"; then
    echo "FAIL $name: does not typecheck on this compiler"
    grep -vE '^\[security\]' <<<"$out" | head -5
    failed=1
    continue
  fi
  echo "ok $name (via $symbol)"
done

if [[ "$failed" -ne 0 ]]; then
  echo "published stdlib typeck failed" >&2
  exit 1
fi
echo "all published stdlib packages typeck"
