# Native layout fundamentals — issue 1179

Stacked on PR 1178 (`codex/native-runtime-1177`). Companion DSC contract: issue
309, PR 310, revision ecce328406280ab3415ec0b1c28a6f6303dc1357.

## Scope

Shared typed box styles, source-order utility resolution, flex alignment and
sizing, edge spacing, basic fontdue text wrapping, adjacent text-run coalescing,
and rectangular clipping shared by layout scenes, native painting, WebGL and
hit testing. All layout coordinates are logical pixels. The React host's
synthetic wrapper is removed for a single application root so viewport-relative
root dimensions work as in the restricted compiler/WASM host.

The root canvas stays white; a sized root's background paints its actual box
rather than flooding the whole viewport. Default flex shrink is now 1. Full
behaviour and exclusions are in docs/dekascript/native-layout.md.

## Evidence

- Native preview tests: 2 existing host tests plus 4 numerical layout tests pass.
  Covers centred boxes, fractions/limits, independent edges, grow/shrink/wrap,
  text runs and wrapping, scale-invariant geometry, paint clips and hit regions.
- Real normal-DSC/Deka/V8/React integration: 4 tests pass, including state-driven
  alignment, changed hit coordinates, wrapped text and restoration.
- 16 scenes agree between a native binary and the actual wasm-bindgen WASM
  binary at 1x/2x. Comparison includes node bounds, paint, clipping, targets and
  glyph RGBA data (numeric tolerance 0.0001). This tests the shared renderer,
  not full browser React execution or GPU pixel equality.
- Mutation proof: removing the ancestor-clip check from hit testing makes the
  clipping regression fail. Restoring it passes. DSC separately proves its
  style test fails when items-center is ignored.
- CI gains the native/WASM scene comparison with mandatory sccache. The website
  companion adds actual Playwright pointer/keyboard/resize and canvas-pixel checks.

## Boundaries

No kit controls, editable text, complex shaping, scrolling, Grid, CSS cascade,
new tag vocabulary or persistent Rust tree protocol. Existing input timing
(native down/browser up) remains explicitly outside this layout slice. The
browser still executes the restricted native program, not the full React host.

-codex
