# Native WASM/WebGL experiment — #1173

Depends on native UI PR #1172 and DSC PR #308. Website integration is website#191.

- Extracted ProgramApp so browser and native development hosts share state/evaluation/reload rules.
- Shared Taffy layout, bundled font glyph rasterization, scene commands, hit testing and focus painting.
- GPUI now consumes the portable scene through a canvas; WebGL consumes it through the website adapter.
- Added wasm-bindgen host with compilation, state-preserving edits, reset, input and error retention.
- Added controlled Rust/V8 backend size comparison; measurements and reproduction are in docs/dekascript/native-backend-cost.md.

Validation: workspace/all-target check; workspace/all-target/all-feature Clippy (existing unrelated warnings, none in changed crates); seven focused Rust tests; wasm32 release build; native release window visibly rendered the scene. Disabling hit testing makes the new event regression fail; restored implementation passes. The website additionally tests the shipped WASM bytes and existing tour behavior. No claim of pixel-identical rendering, full CSS/DOM support, full npm compatibility, or workload memory characterization.

Sami explicitly deferred visual polish/parity work: focus remains the basic compiler/runtime/render/input machinery. No merge, deployment or release.

-codex
