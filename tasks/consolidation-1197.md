# Native source consolidation and historical corpus retirement (#1197)

User decisions: milestone 0.60.0; bring the needed DSC source into Deka; retire
standalone testsuite machinery; retain Rust implementation tests and the public
`deka test` app-testing command. Website showcase work is website#197.

## Source ownership

- `crates/deka_syntax`: lexer, parser, checker and original Rust tests from DSC
  ff1d486813088df977aa5c3fe5400995ec67a710.
- `crates/deka_native_ir`: shared node/style model from that same DSC commit.
- `crates/deka_native_ui`: Taffy/fontdue scene, layout and animation renderer from
  Deka fd8e705d9622e19bf19fd78ad963971c15925c2d.
- `crates/deka_vm`: moved from `experiments/deka-vm`; all three dependencies are
  local paths. No compiler subprocess or JavaScript emitter in this route.
- `tools/deka-package`: moved build-time bundler. Separate workspace keeps Tauri
  packaging dependencies out of the shipped VM binary.

`native-source-imports.json` records source commits and original file hashes.
The import preserves original source and notices; it does not graft entire DSC
Git history. Earlier repository history remains available at the recorded commits.

## Retired in this change

External corpus download/target and runner; tests only for that retired runner;
four-repository lockstep gate; corpus packing/dump release job; corpus manifest
field; testsuite-site release dispatch. `deka test`, its library and CLI behavior
are preserved. Tour pairing tests continue through the retained tour target.

Local native tests and relocated-app packaging now gate canary publication.
Promotion still checks successful validation for the exact source commit.
No repository has been archived, no release has been triggered.

## Validation evidence

- 569 native frontend/IR/renderer/VM tests pass.
- 17 CLI command/app-testing checks pass with the checksum-verified pinned DSC.
- Four retained content-target unit tests pass. Re-enabling the retired lookup
  makes the retirement test fail; restoring the change passes.
- Workspace check and all-feature clippy pass; imported compiler warnings and
  legacy warnings remain. VM and packager strict targeted clippy pass.
- Relocated packaged-app test passes, including missing-payload failure.
- Changed workflow lint and formatting checks pass.

## Remaining migration boundaries

These prevent claiming DSC is safe to archive today:

- Legacy CLI check/fmt/transpile/LSP and V8 execution still use DSC.
- Formatter/LSP/module-resolution integration needs their relevant DSC crates;
  only native compilation's required frontend is imported so far.
- `deka test` still uses the legacy runner and `@deka/test` implementation.
  Reimplement/port on the VM, including real application imports, before calling
  the new runtime a complete app-testing toolchain.
- Website#197 must replace old JS/HTML lesson execution with the new VM WASM
  adapter. Existing native-preview WASM is an earlier restricted interpreter.
- npm delivery, editor tooling, website compiler sync and stdlib releases still
  need a downstream consumer audit before external DSC is retired.
- Existing release builds still publish the legacy CLI. Runtime validation added
  here is additional evidence, not a claim that the shipped CLI has switched.
- Browser/native parity, supported language surface and public module host
  contracts must be demonstrated, not inferred from the renderer import.

The pipeline conversation with Sami follows the tour demo. No new deployment,
auto-promotion or repository archival is implied by this change.

-codex
