# Deka Task Runner (Deno-style)

Goal: Implement `deka task` using Deno's cross-platform task shell, with clear security boundaries and documentation. One commit per task with verification noted.

## Tasks

- [x] Task 1: Design + wiring
  - Add `deka task` command skeleton in CLI
  - Define task schema in `deka.json` (`tasks`, string or object form)
  - Decide `scripts` compatibility behavior
  - Verification: `cargo build --release -p cli`
  - Commit: `feat(cli): add task command skeleton and schema`

- [x] Task 2: Task execution engine
  - Integrate `deno_task_shell` in CLI
  - Implement task runner (CWD = `deka.json` dir, `INIT_CWD` set)
  - Map `deka` to current executable for tasks
  - Verification: `cargo build --release -p cli`
  - Commit: `feat(cli): run tasks with deno_task_shell`

- [x] Task 3: CLI routing + UX
  - Decide behavior for `deka run <name>` (route to tasks or hint)
  - Implement wildcard task matching + dependencies (if supported)
  - Verification: `cargo build --release -p cli`
  - Commit: `feat(cli): task routing and wildcard support`

- [x] Task 4: Security semantics
  - Ensure task runner doesn’t use runtime `run` gates
  - Keep runtime `run` gating for user code
  - Add explicit boundary messaging in errors/help
  - Verification: `cargo build --release -p cli`
  - Commit: `feat(security): separate task runner from runtime run-gate`

- [x] Task 5: Tests + docs
  - Add task runner tests (glob, built-ins, deps, INIT_CWD)
  - Update docs for `deka task` and policy separation
  - Verification: `cargo test -p cli --release`
  - Commit: `docs/test: task runner behavior`

- [x] Task 6: Parallel dependency execution
  - Run task dependencies concurrently
  - Deduplicate dependency execution across task runs
  - Verification: `cargo test -p cli --release`
  - Commit: `feat(cli): run task dependencies in parallel`

- [x] Task 7: Task listing output
  - Add `deka task --list` and `deka task --json`
  - Document task listing usage
  - Verification: `cargo test -p cli --release`
  - Commit: `feat(cli): add task listing output`

- [x] Task 8: Deterministic ordering + json tests
  - Ensure deterministic dependency ordering for parallel runs
  - Add json task listing tests
  - Verification: `cargo test -p cli --release`
  - Commit: `test(cli): deterministic task ordering and json output`
