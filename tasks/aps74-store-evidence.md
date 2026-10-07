# APS 74 shared-store extraction

Base: `origin/main` at `08511f1b76f19f8bda828c631254d59251af6180`.
Branch: `codex/shared-store-aps74`; standalone, no stack.
Approval: https://github.com/dekaruntime/aps/issues/74#issuecomment-6033234596
(the authoritative APS 74 decision comment; see the live issue).

This PR mechanically moves the #1306 retained store, selector matcher and
WireNode style preparation into deka_native_ir. VM module paths re-export the
shared types. Serde wire support is enabled by the VM's optional UI dependency;
the independent packaging workspace lockfile adds that existing serde edge.
No compiler, evaluator, handler execution, renderer, test assertion, fixture,
corpus source or passing-list behavior changes. Record bridge visibility is
widened only for the crate boundary and the unchanged reference tests; raw
resource internals are hidden from docs and remain an unpublished contract.

## Unchanged behavior

`./run.sh` before and after: **exit 0, 1,199 tests** each.
Listed corpus gate before and after: **exit 0, 747 of 747** each.
The archive SHA-256 matches scripts/testsuite-corpus-version:
`18c86aad30c30398ae8678ca5915d104560bbe9a96ff9b87ba0ccf4abf1f3d95`.

Run the move proof on this standalone extraction branch; subsequent UI PRs
add shared-store authoring APIs.

`tasks/evidence/aps74-store/verify_move.py` compares the moved implementation
against the clean base after normalizing only visibility, imports, removed
VM-only cfg gates and rustfmt whitespace/trailing commas. It also compares
all original store/selector test bodies byte-for-byte and checks that the VM
integration tests, tour fixtures and corpus list have no diff. **Exit 0.**
There is one store and style preparation implementation. This is a move with
no new behavior, so a failing literal behavior revert is not applicable; the
clean base and moved source both pass the same gates.

## Commands and evidence

All Rust commands run serially in release mode using machine sccache,
`TMPDIR=/Volumes/Projects/codex/.tmp-deka` and checkout-local `.target`.
Full logs and explicit exit files are under `tasks/evidence/aps74-store/`.

- `cargo check --release --locked --workspace --all-targets`: 0.
- `cargo clippy --release --locked --workspace --all-targets --all-features`: 0;
  existing warnings in untouched crates/files, no warnings in moved code.
- `cargo clippy --release --locked -p deka_native_ir --all-targets --all-features --no-deps -- -D warnings`: 0.
- `cargo fmt -p deka_native_ir -p deka_vm --check`: 0.
- `cargo build --release --locked -p deka_cli --no-default-features -p corpus_gate`:
  before/after 0; the resulting release binaries run the gates.
- `cargo check --release --locked --manifest-path tools/deka-package/Cargo.toml --all-targets`: 0.
- `cargo build --release --locked -p deka_vm --no-default-features --features gpu --bin dvm-ui`: 0.
- `cargo build --release --locked --manifest-path tools/deka-package/Cargo.toml`: 0.
- `python3 tools/deka-package/test_bundle.py .target/release/dvm-package .target/release/dvm-ui crates/deka_vm/examples/packaged-counter/deka.json /Volumes/Projects/codex/.tmp-deka/aps74-package-tests`: 0.
  Both relocated apps execute their real handlers after source/staging deletion;
  metadata, resources, signature and missing-payload failure checks pass.

Extra strict VM clippy with warnings denied exits 101 on this host's Clippy
1.99: two pre-existing chunks_exact_to_as_chunks warnings in untouched
`crates/deka_vm/src/bytes.rs` (lines 36 and 68). Workspace clippy exits 0;
no lint allowances or unrelated VM edits were added. Initial cross-crate
visibility and the expected packaging lock refresh were fixed before final
validation; failed intermediate logs are retained where useful.

Docs: `docs/rust-ui/retained-store.mdx`.
No merge, approval, tag, publishing, release, force push, CI polling or helpers.

-codex
