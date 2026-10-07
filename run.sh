#!/usr/bin/env bash
# Run the in-repository native frontend, VM and renderer tests.
# App authors use `deka test`; this script validates Deka itself.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")"
exec cargo test --locked --release -p deka_syntax -p deka-fmt -p deka_native_ir -p deka_native_ui -p deka_vm -p deka_native_web --features deka_vm/compiler,deka_vm/host,deka_vm/ui "$@"
