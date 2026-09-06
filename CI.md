# CI and the build system

How CI actually runs, and how to debug it when it breaks. For test commands see
`TESTING.md`; for day-to-day dev see `REPO.md`; for releases see `RELEASE.md`.

CI runs on **self-hosted runners on Sami's machines**, not GitHub-hosted ones.
That is the single most important fact on this page. Self-hosted runners carry
state between jobs, share hardware with interactive work, and fail in ways that
GitHub-hosted runners never do — including several that produce error messages
pointing at entirely the wrong thing.

---

## Rule zero: establish WHERE before investigating WHY

Before reading a single log line, find out which machine ran the job:

```bash
gh api repos/dekaruntime/deka/actions/runs/<run-id>/jobs \
  --jq '.jobs[] | "\(.name): \(.runner_name)"'
```

This is not optional. In August 2026 a CI outage took hours longer than it
should have because the investigation happened on the wrong host the entire
time. Every runner has the same `~/actions-runner/` path, so a log excerpt tells
you nothing about which machine produced it, and an ssh session you already have
open is not evidence.

## The runner fleet

| Runner | OS / arch | Repos | Notes |
|---|---|---|---|
| `bugsy` | macOS ARM64 | deka, website, testsuite | Runs **three** runner installs |
| `imac` | macOS X64 | deka | Also Sami's interactive machine |
| `thinkpad` | Linux x64 | deka | Usually the least loaded |

Which job lands where is decided by `runs-on` labels in the workflow:

| Job (`ci.yml`) | `runs-on` | Lands on |
|---|---|---|
| Detect changed files | `[self-hosted, linux, x64]` | thinkpad |
| **Rust tests** | `[self-hosted, macOS, ARM64]` | **bugsy** |
| Lockfile consistency | `[self-hosted, linux, x64]` | thinkpad |
| File-size gate | `[self-hosted, linux, x64]` | thinkpad |
| Utility CSS generator | `[self-hosted, macOS, ARM64]` | bugsy |

The heavy job — `Rust tests` — runs on bugsy. If you are debugging a Rust test
failure, bugsy is the machine you want.

That job installs the published **dsc** compiler from
`https://dsc-wasm.deka.gg` (`scripts/ci-install-dsc.sh`) and runs
`deka check` / `fmt` / `transpile` with `DEKA_DSC` set. Tour, conformance,
and `deka run` still use in-process `deka_compile` until isolate compile
through dsc is solid. Do not put `dsc` next to `target/release/cli` in CI:
the isolate loader will pick it up as a sibling and fail module fixtures.

## Where the logs really are

When a job dies without completing, **GitHub never receives its logs**.
`gh run view --log` returns `BlobNotFound` / HTTP 404, which looks like a
tooling problem and is actually a signal: the job died hard.

The only source in that case is the runner host:

```bash
ssh <runner-host>
ls -lt ~/actions-runner/_diag/Worker_*.log | head    # newest = your run
ls -lt ~/actions-runner/_diag/Runner_*.log | head    # listener-level
```

Match by timestamp — worker log filenames encode the UTC start time. These logs
are verbose (10k+ lines is normal) but they contain the real exception.

## Known failure modes

Each of these has a misleading symptom. Read the right-hand column before
believing the error text.

### Duplicate runner processes → `Job not found`

**Symptom:** the job runs for a minute or more doing real work, then fails with

```
POST .../completejob failed. HTTP Status: NotFound
TaskOrchestrationJobNotFoundException: Job not found ... workflow instance not found
```

**Cause:** two `Runner.Listener` processes registered to the same agent ID —
typically the `launchctl` service plus a stray manual `./run.sh`. They race for
jobs, so one instance claims a job while the other executes it, and the executor
finds its record gone when it reports completion.

**Check:**

```bash
pgrep -fl Runner.Listener      # on the runner that ran the job
```

More than one line for the same install is the bug. Kill the manual one and
restart the service. A stray `./run.sh` can sit there for days without anyone
noticing.

### Node 20 actions → `could not read Username`

**Symptom:** checkout logs `Setting up auth`, then fails every fetch retry with

```
fatal: could not read Username for 'https://github.com': terminal prompts disabled
```

**Cause:** not credentials. A Node 20 action being force-run on Node 24 (the
runner logs `This workflow is running with Node 24 by default`) whose auth setup
no longer takes effect.

**Confirm it is not credentials** by running the same fetch by hand on the host:

```bash
cd ~/actions-runner/_work/deka/deka
git fetch --no-tags --depth=1 origin '+refs/pull/<N>/merge:refs/remotes/pull/<N>/merge'
```

If that succeeds, credentials are fine and the action version is the problem.

**Keep every action on a Node 24 release.** As of 2026-08:

| Action | Minimum Node 24 version |
|---|---|
| `actions/checkout` | v5 |
| `actions/upload-artifact` | v6 |
| `actions/download-artifact` | **v7** (v6 is still Node 20) |

`checkout@v5` requires runner ≥ 2.327.1. Check with
`ls -ld ~/actions-runner/bin` — it symlinks to `bin.<version>`.

### Dirty runner workspace

The workspace persists between jobs. `ci.yml` runs
`cargo update --workspace --locked`, which can leave `Cargo.lock` modified;
`actions/checkout` may then fail against the dirty tree.

```bash
cd ~/actions-runner/_work/deka/deka && git status --porcelain   # expect empty
```

Clean with `git checkout -- . && git clean -fd`.

### Concurrency cancellation

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
```

Any new event on the same ref kills the in-flight run. Re-triggering repeatedly
produces a cascade of cancelled runs that reads as a persistent failure. Trigger
once, then leave it alone.

### Resource contention

The runners share hardware with interactive work. Load average 19 has been
observed with CI `rustc` at 210% competing against a local `next-server` at
139%. If a job dies mid-build with no clear error, check `uptime` and
`ps aux | sort -k3 -rn | head` on the runner before assuming a code problem.

## Re-triggering CI

**`gh run rerun` does not work on this setup.** It consistently fails at
checkout because credentials are not re-provisioned for the re-run. It looks
exactly like a persistent test failure.

Use, in order of preference:

1. **Push a commit** to the branch — cleanest, always works.
2. **Close and reopen the PR** — works, but fires a fresh `pull_request` event;
   do not do it while a run is in flight or the concurrency group cancels it.

## Before blaming CI, reproduce locally

CI's exact sequence (see `ci.yml`):

```bash
cargo build -p deka_http -p pool -p engine -p deka_js -p php-rs -p bundler
cargo test -p deka_http
cargo test -p pool
cargo test -p engine
cargo test -p deka_js
cargo test -p bundler
cargo test --locked -p cli --lib -- --test-threads=1
scripts/test-deka-compiler-wasm.sh
cargo build --release -p cli
./run.sh --skip-build
```

**`TESTING.md`'s crate list is not the same as CI's.** It omits
`deka_compiler_wasm`, which CI covers via `scripts/test-deka-compiler-wasm.sh`.
Following the documented process can give a green local run and a red CI — this
has happened. When verifying a change against CI, use the list above.

`cargo test -p php-rs` is deliberately absent from CI. `php-rs` has a large
number of pre-existing failures; do not treat its local red as a regression
without diffing failure **names** against a clean `origin/main` worktree.

## Debugging checklist

Work top to bottom. Most of these take seconds and each eliminates a class of
cause.

1. Which runner ran it? — `gh api .../jobs --jq '.jobs[].runner_name'`
2. Which step failed? — `gh api .../jobs --jq '.jobs[] | .steps[] | "\(.name): \(.conclusion)"'`
   (`null` conclusions after a success mean the job died there without reporting)
3. How long did it run? — a job dying in under ~20s did not reach the tests
4. Duplicate runners? — `pgrep -fl Runner.Listener` on that host
5. Workspace clean? — `git status --porcelain` in `_work/<repo>/<repo>`
6. Disk and load? — `df -h`, `uptime`
7. Worker log on the host — `~/actions-runner/_diag/Worker_*.log`, newest by time
8. Only now: reproduce locally with the sequence above
