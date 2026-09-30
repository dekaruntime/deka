# Deka repository guide

Deka is consolidating its native frontend and runtime here for the 0.60.0
milestone. The legacy JavaScript CLI remains in the workspace while its commands
and downstream consumers migrate. Do not confuse a working native preview with
completion of that migration.

## Native source

| Path | Responsibility |
| --- | --- |
| `crates/deka_syntax` | Lexer, parser, typechecker and diagnostics imported from DSC |
| `crates/deka_native_ir` | Node and styling model shared by native hosts |
| `crates/deka_native_ui` | Layout, text, scene generation and animation |
| `crates/deka_vm` | Bytecode compiler, VM, heap, host operations and component state |
| `tools/deka-package` | Build-time Tauri bundler adapter configured by `deka.json` |

These use local path dependencies. The import provenance is in
`tasks/native-source-imports.json`; migration status is in
`tasks/consolidation-1197.md`. The VM's comparison-only V8 feature is optional,
and the shipped native example does not enable it.

## Build and validate

```sh
cargo build --locked --release -p deka_vm --features compiler,host,ui
./run.sh
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets --all-features
```

On supported desktop hosts, build the renderer with
`cargo build --locked --profile native -p deka_vm --features gpu --bin dvm-ui`.
The `native` profile preserves the preview's size optimization, LTO and stripping
without changing the legacy CLI's release profile.
Packaging commands and relocation checks are in `tools/deka-package/README.md`.
See [TESTING.md](TESTING.md) for runtime versus app testing.

## Legacy CLI and downstreams

`crates/cli` composes the shipped legacy CLI. `crates/run`, `runtime`, `engine`,
`deka_host` and related crates still implement its JavaScript execution path.
`crates/deka_test` implements public app testing (`deka test`) and stays in the
product. `crates/self_cmd` retains checksummed tour fetching and maintenance.
The retired external corpus is no longer a fetch target or release prerequisite.

Build the legacy CLI with `cargo build --release -p cli`. Its compiler-dependent
tests need the published DSC pinned in `scripts/dsc-version`; install it with
`scripts/ci-install-dsc.sh`. Native VM tests do not need that executable.

The website owns the tour UI and browser verification. Its migration is tracked
in website#197. Compiler/VM/renderer browser artifacts must be a compatible set;
old JavaScript compiler WASM and earlier restricted-interpreter previews are not
proof of the new VM's browser behavior.

## Releases

[VERSIONING.md](VERSIONING.md) describes the consolidation boundary.
[RELEASE.md](RELEASE.md) and [PUBLISH.md](PUBLISH.md) describe the existing canary
and promotion workflows. The external corpus's lockstep/version/site cascade is
retired. The remaining legacy DSC artifact dependency is explicit until migration.

Agents submit PRs; humans own merge and promotion. No archival, deployment or
release is implied by a local source import or a green test run.
