# Testing Deka and Deka applications

Deka owns the quality of its runtime. App developers use `deka test` to test
DekaScript applications against that runtime. Both are product requirements.

The historical standalone `dekaruntime/testsuite` corpus, its aggregate score,
four-repository version gate and site deployment cascade are retired. They are
not prerequisites for building or releasing the native runtime. Historical dump
sources remain in Git and some legacy browser regression tools remain under
`tests/dump`; that directory is not the native runtime's test authority.

## Runtime contributors

The native frontend, VM and renderer are local workspace crates. From this repo:

```sh
./run.sh
```

This runs release-mode Rust tests in `deka_syntax`, `deka_native_ir`,
`deka_native_ui` and `deka_vm`, including actual DekaScript compilation,
execution, host calls, async completion/cancellation, closure/heap lifetime,
component events, dynamic lists and UI frames. It needs no corpus download,
external DSC executable or expected-failure list. Extra arguments go to Cargo.

The desktop workflow also packages an app, relocates it, removes the source,
executes its real VM handlers, checks its assets/signature and verifies that a
missing payload fails. See `tools/deka-package/README.md`.

`tools/deka-package` is a separate Cargo workspace with its own lockfile. When
a native runtime dependency changes, update and commit that lockfile too:

```sh
cargo update -p deka_vm --manifest-path tools/deka-package/Cargo.toml
cargo check --locked --manifest-path tools/deka-package/Cargo.toml --all-targets
```

The root workspace checks do not validate this packaging workspace. Its locked
build and relocated-app test are required evidence for native dependency changes.

Before a Rust PR is pushed, run:

```sh
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets --all-features
```

Run targeted behavioral tests for the change. Demonstrate that a regression test
fails when the fix is removed. Do not replace assertions with snapshots of source
strings or allow a skipped/empty run to masquerade as validation.

Legacy CLI and application-test coverage remains under `crates/cli/tests` and
`crates/deka_test`. The CLI still has DSC/V8 consumers during the migration;
its tests require the pinned compiler installed by `scripts/ci-install-dsc.sh`.
The native crate tests above do not have that dependency.

## Language migration corpus gate

The native-runtime workflow also runs the pinned historical corpus while old
language features are being ported. `tests/corpus-passing.txt` lists programs
that must keep matching. Negative cases must fail at the expected stage with
the expected diagnostic text, not merely exit nonzero. The gate supplements
missing diagnostic guards in the pinned corpus for the eight note-05 cases;
explicit corpus metadata takes precedence.

Each staged project first runs `deka check`. A check refusal is a compilation
failure and the program is never executed. After a successful check, the gate
runs `deka run` and records any failure as a runtime failure, even if it prints
nothing. Only the program's output is compared with its expected stdout; the
check command's success message is not part of that output. Both child output
pipes are drained during each command, so large output cannot fill a pipe and
turn a finished program into a timeout. Regression tests cover observed phases
and actual CLI execution, alongside the listed corpus gate.

`corpus-gate --all` is an inventory command: it exits zero even with unmatched
cases. Use the listed-case gate for validation and compare inventories when
adding passing programs. Never count an unsupported feature's failure as the
intended runtime error of that feature.

## App developers

`deka test [files...]` and its `--test-name-pattern` / `-t` filter remain part of
the CLI. Test discovery, assertions, failure reporting and a nonzero exit on
failure belong to this command, not to a separate website or corpus repository.
For npm installations invoke it through `npx deka test` or a package script.

The existing command currently executes through the legacy runtime. Porting its
execution and `@deka/test` library to the new VM is tracked in #1197. Removing the
old corpus does **not** mean that port has already shipped or that application
assertions can be removed.

## Browser and release evidence

The website owns browser tests for its tour. They must exercise the same compiled
runtime and renderer as native demos: real source edits, event dispatch and
resulting scene/output. Browser screenshots complement semantic assertions;
they do not prove native platform input, menus, accessibility or packaging.

Release publication requires the reusable native-runtime workflow and existing
compiled-executable smoke tests to succeed. It then writes a `validation.json`
for that exact commit. Promotion still rejects missing evidence, mismatched
commits and non-green results. Nothing in corpus retirement automatically
promotes or publishes a release.
