# Security Defaults + Prompt Reduction

Goal: reduce excessive runtime security prompts by introducing safe dev defaults and path-prefix allow/deny semantics.

## Tasks

- [x] Task 1: Path-prefix allow/deny for read/write/wasm
  - Treat allow/deny path entries as directory prefixes
  - Keep exact matching for env/net/run/db
  - Add unit tests for prefix behavior
  - Verification: `cargo test -p cli --release`
  - Commit: `feat(security): path-prefix rules for read/write/wasm`

- [x] Task 2: Dev-mode defaults
  - On `--dev`, auto-allow:
    - read: project root (prefix)
    - write: `./.cache`, `./php_modules/.cache`
    - wasm: all
    - env: all
  - Respect explicit deny rules
  - Verification: `cargo test -p cli --release`
  - Commit: `feat(security): apply dev defaults for serve`

- [x] Task 3: Docs
  - Document prefix behavior and dev defaults
  - Verification: `cargo test -p cli --release`
  - Commit: `docs(security): clarify dev defaults and path rules`
