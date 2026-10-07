#!/usr/bin/env bash
# Headless Rust UI behavior, markup diagnostics and complete tour scene parity.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
exec cargo test --locked --release --no-default-features --features tour,web -p deka-ui -p deka-ui-macros "$@"
