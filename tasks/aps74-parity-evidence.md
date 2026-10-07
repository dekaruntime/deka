# APS 74 complete-scene tour gate

Step 3 stacks on macros PR #1415. All 27 Rust twins are ordinary runnable
examples under `crates/deka_ui/examples/tour/`, with explicit Cargo targets.
All original fixtures, VM tests, corpus list and VM source remain unchanged
after the permitted standalone extraction #1414.

The gate renders the VM and Rust UiApp through the same Renderer::render_at
implementation. It compares complete JSON scenes with exact field/value
equality. Glyph images are sorted by id because their cache is an unordered map;
their metadata and every pixel byte remain in the comparison. No ids, text,
numbers, booleans, style/layout, animation flags, targets or paint are omitted.
The diagnostic walks an unequal scene only to report its first differing field.

One registry generates both the lesson inventory and all 27 tests. The inventory
requires the original 27 files, their Rust twins and explicit runnable targets.
Every handler is exercised using the same real point through transformed and
clipped scene hit tests. Three viewport/scale/reduced-motion configurations
cover rest, same clicks and fixed times. Motion checks include reversal before
completion, exit/re-entry, stagger delay boundaries, keyframe repeat/alternate,
spring movement and settling. Original run.sh remains unchanged; a separate
release-only headless script runs Rust UI behavior, diagnostics and parity in
the existing native CI workflow.

A literal edit changes the Rust counter's actual increment from 1 to 2.
The complete-scene gate fails at the first event (100ms), exit 101; compilation
succeeds. The proof script restores source in finally. Final restored gate,
workspace checks/clippy, strict touched-crate clippy, original native tests,
formatting and runnable counter evidence live in `tasks/evidence/aps74-parity/`.
Final tour result: 28 tests, all 27 lessons, 81 configurations and 1,032
complete-scene comparisons pass. The headless Rust UI script passes 61 tests.
Workspace check/clippy, strict touched-crate clippy, formatting, the original
1,199-test native reference suite, the runnable counter exercise and production
graph/source-diff guards all exit 0.
There are no legitimate scene differences and no weakened comparisons.

Own docs: `docs/rust-ui/tour-parity.mdx`. Only test dependencies link the VM;
the production Rust UI library graph remains compiler/VM-free.

-codex
