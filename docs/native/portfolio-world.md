# Portfolio world experiment

Fernwood is an original, small top-down town. Walk with WASD or arrow keys, approach a house, and press Enter. Enter inside returns to town; walking through the bottom doorway also exits. Each of the three houses uses a different project description. M toggles desktop audio. The browser provides sound and touch controls.

Run from this experimental checkout:

```sh
CARGO_TARGET_DIR="$PWD/.target" TMPDIR="$PWD/.tmp" cargo run --locked --release -p deka_native_ui --example portfolio-world --features world-audio,runtime-shaders
```

Use `-- --muted` or `-- --reduced-motion` for those preferences. `runtime-shaders` is needed on Macs without the Metal compiler; other GPUI platforms can use `world-audio` alone. Linux audio requires the ALSA development libraries used by CPAL. The verified desktop target for this slice is macOS; Linux and Windows have not been exercised.

The website hosts this scene at `/tour/world`. Browser sound starts after Enter the world. Loss of focus clears held keys and suspends audio; hidden tabs stop rendering. Audio failure leaves the world playable and reports an explanation.

## What is shared

`deka_native_ui::world` owns the fixed 120 Hz simulation, collision footprints, camera, depth sorting, sprite artwork, room changes, fade timing, text layout and PCM synthesis. The world emits the same `Scene` as the UI renderer. GPUI uses the same desktop painter as existing UI examples; the browser uploads those commands with WebGL. Browser CSS only styles the surrounding website and controls. Sprite textures use nearest filtering in WebGL; desktop filtering is platform-owned, so pixel-identical screenshots across GPU adapters are not promised.

The world map is 960 by 640 logical units. Houses and tree trunks have collision footprints; roofs and foliage can cover the character without blocking the same area. Feet determine object draw order. Diagonal movement is normalized. Frame gaps are capped at 100 ms to avoid teleportation on resume; the explicit monotonic clock permits deterministic tests. Reduced motion removes fades and outdoor walking bob; movement remains directly controlled.

Audio is an original eight-second looping synthesized melody plus footstep/door effects at 22,050 Hz mono. Web Audio and optional Rodio/CPAL are playback adapters for the same Rust samples. No decoder, remote audio or external artwork is required.

## Scope

This is a renderer/world proof of concept, not a finished game engine or a DekaScript world component. Project data currently lives in the Rust `PROJECTS` array; recompilation is needed to change it. React hosts the browser canvas and controls, but does not run the simulation. No physics, networking, saved progress, sprite editor, project URL activation or semantic accessibility tree is implemented. The page includes keyboard instructions and ordinary browser controls; world content is still canvas-only.

The transport still serializes paint commands and visible texture bytes per frame. It is intentionally simple and suitable for this small example; a production world should retain texture handles and batch sprite draws. Do not infer a production game performance or bundle-size claim from this demo.

Tests cover blocked movement, diagonal speed, frame cadence, focus release, depth ordering, room entry/exit, reduced motion, bounded clocks and PCM/mute behavior. `deka_native_web`'s `world-scenes` example emits native results for the website's `scripts/test-native-world.mjs` to compare against shipped WASM. Playwright exercises the real keyboard, canvas and audio host.
