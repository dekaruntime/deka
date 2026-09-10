#!/usr/bin/env bash
#
# Run this checkout's language suite: tour, Hats snippets (`deka run`), ADHOC.
#
#   ./run.sh                 build the CLI, compile every tour lesson, run Hats
#   ./run.sh --filter json   subset (id / slug / title)
#   ./run.sh --list          list lessons and fixtures, do not run
#
# This is the in-tree replacement for dekaruntime/testsuite's ./run.sh as the
# language gate. Browser/WASM stays the live playground on testsuite.deka.gg.
# See TESTING.md and deka#292.
#
# Everything the run depends on is installed or verified here, because each of
# these has silently produced a confident wrong answer:
#
#   stale native build  a binary older than its own tree grades your code with
#                       an old compiler.
#   bun not on PATH     non-login shells on the fleet macs do not have bun.
#   missing fixtures    a partial checkout "passes" zero tests.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
REPO_ROOT="$PWD"

FILTER=""
LIST=0
SKIP_BUILD=0
JOBS=""
UPDATE_KNOWN_FAIL=0
LOCKED=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --filter|-f)
      FILTER="${2:-}"
      [[ -n "$FILTER" ]] || { echo "fatal: --filter needs an argument" >&2; exit 2; }
      shift 2
      ;;
    --filter=*)
      FILTER="${1#*=}"
      shift
      ;;
    --jobs|-j)
      JOBS="${2:-}"
      [[ -n "$JOBS" ]] || { echo "fatal: --jobs needs an argument" >&2; exit 2; }
      shift 2
      ;;
    --jobs=*)
      JOBS="${1#*=}"
      shift
      ;;
    --locked)               LOCKED=1; shift ;;
    --list|-l)              LIST=1; shift ;;
    --skip-build)           SKIP_BUILD=1; shift ;;
    --update-known-fail)    UPDATE_KNOWN_FAIL=1; shift ;;
    -h|--help)              sed -n '2,18p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)
      printf '\n\033[31mfatal\033[0m unknown argument: %s\n' "$1" >&2
      printf '      fix: ./run.sh --help\n' >&2
      exit 2
      ;;
  esac
done

say()  { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() {
  printf '\n\033[31mfatal\033[0m %s\n' "$1" >&2
  if [[ $# -gt 1 ]]; then
    printf '      fix: %s\n' "$2" >&2
  fi
  printf '\nEnvironment is not fit to run. This is NOT a result for your runtime.\n' >&2
  exit 2
}

# --------------------------------------------------------------------- bun ---
if ! command -v bun >/dev/null 2>&1; then
  for candidate in "$HOME/.bun/bin" /usr/local/bin /opt/homebrew/bin; do
    [[ -x "$candidate/bun" ]] && PATH="$candidate:$PATH" && break
  done
fi
command -v bun >/dev/null 2>&1 \
  || die "bun is not installed or not on PATH" \
         "install from https://bun.sh, or add ~/.bun/bin to PATH"
say "bun $(bun --version)"

# ---------------------------------------------------------------- fixtures ---
[[ -f tests/tour/manifest.json ]] \
  || die "tests/tour/manifest.json is missing" \
         "this is a deka checkout; tour lessons live in tests/tour/"
TESTSUITE_ROOT="${DEKA_TESTSUITE_ROOT:-$REPO_ROOT/.cache/testsuite-corpus}"
[[ -d "$TESTSUITE_ROOT" ]] \
  || die "testsuite corpus is missing: $TESTSUITE_ROOT" \
         "run scripts/ci-fetch-testsuite-corpus.sh or set DEKA_TESTSUITE_ROOT"

tour_count=$(find tests/tour -maxdepth 1 -name '*.ds' | wc -l | tr -d ' ')
[[ "$tour_count" -gt 0 ]] \
  || die "tests/tour has no .ds lessons" \
         "add tests/tour/<id>.ds and a row in tests/tour/manifest.json"

suite_cats=$(find "$TESTSUITE_ROOT" -mindepth 1 -maxdepth 1 -type d ! -name '.*' | wc -l | tr -d ' ')
[[ "$suite_cats" -gt 0 ]] \
  || die "testsuite corpus has no category folders" \
         "Hats layout is corpus/<category>/<name>/"

say "fixtures: tests/tour ($tour_count lessons)  testsuite ($suite_cats categories)"

# ----------------------------------------------------------------- native ---
if [[ -n "${DEKA_NATIVE:-}" ]]; then
  [[ -x "$DEKA_NATIVE" ]] \
    || die "DEKA_NATIVE is set but not executable: $DEKA_NATIVE" \
           "unset DEKA_NATIVE to build this tree, or point it at target/release/cli"
  say "native CLI: $DEKA_NATIVE (DEKA_NATIVE)"
elif [[ "$SKIP_BUILD" -eq 1 ]]; then
  DEKA_NATIVE="$REPO_ROOT/target/release/cli"
  [[ -x "$DEKA_NATIVE" ]] \
    || die "no CLI at $DEKA_NATIVE and --skip-build was set" \
           "cargo build --release -p cli   (or omit --skip-build)"
  say "native CLI: $DEKA_NATIVE (--skip-build)"
else
  command -v cargo >/dev/null 2>&1 \
    || die "cargo not found, needed to build the runtime" \
           "install rust from https://rustup.rs"
  say "building native CLI"
  cargo build --release -p cli
  DEKA_NATIVE="$REPO_ROOT/target/release/cli"
  [[ -x "$DEKA_NATIVE" ]] \
    || die "native CLI missing after a successful build: $DEKA_NATIVE" \
           "check CARGO_TARGET_DIR is not redirecting the build elsewhere"
  say "native CLI: $DEKA_NATIVE"
fi

export DEKA_NATIVE

# --------------------------------------------------------------------- run ---
mkdir -m 0755 -p .cache tests/tour/.run-tmp
REPORT="$REPO_ROOT/.cache/report.txt"
: > "$REPORT"

tour_cmd=(bun tests/tour/run.mjs)
suite_cmd=(bun "$TESTSUITE_ROOT/run.mjs")
adhoc_cmd=(bun tests/adhoc/run.mjs)
if [[ -n "$FILTER" ]]; then
  tour_cmd+=(--filter "$FILTER")
  suite_cmd+=(--filter "$FILTER")
  adhoc_cmd+=(--filter "$FILTER")
fi
if [[ "$LIST" -eq 1 ]]; then
  tour_cmd+=(--list)
  suite_cmd+=(--list)
  adhoc_cmd+=(--list)
fi
if [[ -n "$JOBS" ]]; then
  suite_cmd+=(--jobs "$JOBS")
fi
if [[ "$LOCKED" -eq 1 ]]; then
  suite_cmd+=(--locked)
  adhoc_cmd+=(--locked)
fi
if [[ "$UPDATE_KNOWN_FAIL" -eq 1 ]]; then
  suite_cmd+=(--update-known-fail)
fi

tour_status=0
suite_status=0
adhoc_status=0

say "tour: ${tour_cmd[*]}"
echo
set +e
"${tour_cmd[@]}" 2>&1 | tee -a "$REPORT"
tour_status=${PIPESTATUS[0]}
set -e

echo
say "testsuite: ${suite_cmd[*]}"
echo
set +e
"${suite_cmd[@]}" 2>&1 | tee -a "$REPORT"
suite_status=${PIPESTATUS[0]}
set -e

echo
say "ADHOC: ${adhoc_cmd[*]}"
echo
set +e
"${adhoc_cmd[@]}" 2>&1 | tee -a "$REPORT"
adhoc_status=${PIPESTATUS[0]}
set -e

echo
if [[ "$tour_status" -eq 0 && "$suite_status" -eq 0 && "$adhoc_status" -eq 0 ]]; then
  say "suite ran -- tour + testsuite + ADHOC  (full log: .cache/report.txt)"
  exit 0
fi

if [[ "$tour_status" -eq 2 || "$suite_status" -eq 2 || "$adhoc_status" -eq 2 ]]; then
  say "BLOCKED -- environment unfit to run; this is not a result for your runtime"
  say "full log: .cache/report.txt"
  exit 2
fi

say "FAILED  tour=$tour_status  testsuite=$suite_status  ADHOC=$adhoc_status  (full log: .cache/report.txt)"
exit 1
