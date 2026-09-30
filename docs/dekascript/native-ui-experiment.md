# Native UI experiment

This is an experimental target, not a browser-compatible renderer or a released Deka command.
It executes checked Deka source in a native GPUI window without V8 or a webview.

## Build the tools

Use the companion DSC branch for issue 307 and this Deka branch for issue 1171.
Both started at 0.53.7. The final pinned compiler commit is recorded in Cargo.toml.

```sh
# In the DSC checkout:
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
cargo build --release -p deka_native_compile

# In the Deka checkout:
export CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp"
mkdir -p "$TMPDIR"
chmod 700 "$TMPDIR"
cargo build --release -p deka-native --features runtime-shaders
```

`runtime-shaders` enables GPUI's runtime Metal shader compilation, useful on Macs
with command-line developer tools but no Metal compiler. It can add first-window
startup cost. Use `--features gpu` with a complete Xcode toolchain for precompiled
Metal shaders. Linux/Windows support comes from GPUI and remains unverified in
this experiment until those platforms are actually tested.

## Run and edit

```sh
.target/release/deka-native dev examples/native/counter.dsx \
  --compiler /absolute/path/to/dsc/.target/release/dsc-native
```

Edit and save the DSX file. The development host checks the source every 100 ms,
compiles changes in a background worker, and applies validated programs on the UI
thread. The window stays open. Numeric state is preserved when the component
name and ordered state-binding names remain the same. Changing that signature
resets the component. Changing an initializer alone preserves current state.
A failed compilation keeps the last working program and prints diagnostics.

## Produce a native executable

```sh
.target/release/deka-native build examples/native/counter.dsx \
  --compiler /absolute/path/to/dsc/.target/release/dsc-native \
  --out .tmp/counter-production --runtime-shaders

.target/release/deka-native-app
```

Choose a fresh output directory. Generated Rust and a Cargo manifest remain there
for inspection. Cargo builds optimized machine code. The generated application
links `deka_native_ui` and GPUI; it does not link the development interpreter,
source watcher, DSC parser/checker or V8. The same UI values and GPUI adapter are
used in both modes. The builder currently requires this source checkout and a
Rust toolchain; packaging/distribution are outside the first slice.

`--exercise N` on either executable exercises its real first click handler N
times and prints resulting element text without opening a window. This verifies
application semantics, not native pointer routing or rendered pixels.

## Supported first slice

- One exported synchronous zero-argument component, returning JSX.
- Numeric `useState` declarations before the return.
- Nested `div`, `span`, `p`, and `button` elements.
- Literal text and `string(...)` of numbers, state, and `+`, `-`, `*` expressions.
- An inline synchronous button handler containing exactly one numeric setter call.
- Literal utility classes: `flex`, `flex-col`, `flex-row`, `p-N`, `gap-N`, `w-N`,
  `h-N` (N times four logical pixels), `rounded`, `rounded-lg`, `text-sm`,
  `text-base`, `text-lg`, `text-xl`, `text-2xl`, `bg-[#rrggbb]`, `text-[#rrggbb]`.
- Logical-pixel layout, inherited text styles, pointer activation and focused
  Enter/Space activation. Controls/text are custom-drawn through GPUI.

Utilities apply left to right. Containers default to column layout; text containers (`span`, `p`) and buttons
default to row layout. Deliberate spaces next to expressions are preserved. These are explicit prototype rules, not CSS compatibility.
Unsupported elements, attributes, styles, imports, props, effect/async constructs,
state shapes and expressions are diagnosed. General CSS, text inputs, IME,
accessibility semantics, arbitrary modules, multiple components, navigation,
state migration and production packaging are subsequent work.

The example's paper/ink/green palette follows zega.dev's Highland light palette.
This first fixed-palette fixture is not a complete light/dark/system theme implementation.

## Browser teaching

`deka_native_web` combines DSC's portable native compiler with `ProgramApp`, the
same Rust development interpreter used by the native watcher. It exposes source
compilation, frames, pointer events, keyboard events and focus reset through
wasm-bindgen. No GPUI platform code, V8 or JavaScript code generation is linked
into the browser target.

`deka_native_ui::scene::Renderer` owns Taffy layout, inherited text styles,
Atkinson Hyperlegible font rasterization, paint commands and hit targets. The
native GPUI adapter now paints this scene on a canvas instead of constructing
GPUI element layouts. The website's WebGL 2 adapter paints the same rectangles
and glyph images. The font is bundled under its OFL license, included in assets.
The native root background also supplies the canvas background.

```
cargo build --locked --release --target wasm32-unknown-unknown -p deka_native_web
wasm-bindgen .target/wasm32-unknown-unknown/release/deka_native_web.wasm \
  --target web --omit-default-module-path --out-dir .tmp/native-web
```

Use wasm-bindgen CLI 0.2.128. Website issue #191 hosts this at `/tour/native`,
with the canvas replacing the bottom-right RAW JavaScript pane. The tour ships
its own reproducible artifact rebuild script and commit/hash manifest.

This is development-mode source evaluation in WASM. Production native builds
still use emitted Rust without the interpreter or compiler. Sharing a scene
establishes shared layout and glyph data, not a pixel-parity guarantee across
GPU backends. Visual polishing, text wrapping/shaping, accessibility semantics,
scrolling and async application features remain subsequent work. The browser
bridge currently serializes scenes and glyph bytes as JSON; it is intentionally
a proof of concept, not the final buffer/atlas protocol.
