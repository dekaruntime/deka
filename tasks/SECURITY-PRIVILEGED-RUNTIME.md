# Security Privileged Runtime Layer

Goal: Add a privileged runtime security context for internal engine work (compiler cache + lock writes), while preserving userland security gates. One commit per task with verification.

## Tasks

- [ ] Task 1: Design + scaffolding
  - Define privileged context enum in runtime security enforcement.
  - Add helper to detect internal targets (deka.lock, php_modules/.cache/**, .cache/**).
  - Verification: `cargo build --release -p cli`
  - Commit: `feat(security): add privileged security context scaffolding`

- [ ] Task 2: Wire privileged context into runtime
  - Pass privileged context for internal compiler/cache/lock writes.
  - Keep userland enforcement unchanged.
  - Verification: `cargo build --release -p cli`
  - Commit: `feat(security): bypass internal cache/lock writes`

- [ ] Task 3: Tests + docs
  - Add unit tests for internal path detection + enforcement behavior.
  - Update docs to note internal privileged layer (dev/runtime security).
  - Verification: `cargo test -p runtime_core` (and/or relevant tests)
  - Commit: `test/docs: privileged runtime security behavior`
