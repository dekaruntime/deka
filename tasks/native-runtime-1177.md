# Native component runtime — issue 1177

This slice connects ordinary DSC output to the existing native scene renderer through a persistent Deka/V8 isolate. It is separate from the engine comparison recorded in issue 1175 and PR 1176. Base: renderer PR 1174, `codex/ui-webgl-1173` at `67e669b4bb27ee9386b150c8567c171134aaf2d5`.

## Implementation

- `deka-native runtime`: explicit project, normal DSC executable and component export; optional display-free `--exercise`.
- Existing `PhpxEsmLoader`, module grants, host extensions, import-meta operation and security context. Native policy selection uses the existing production-profile resolver or legacy policy parser; development grants do not widen production grants.
- The existing pool bootstrap is extracted without semantic edits so native and request execution share it. Verified against the previous inline string after indentation normalization. Its raw Deno host table remains hidden.
- React 19.1.1 and scheduler 0.26.0 come from existing Deka builtins. React reconciler 0.32.0 is pinned, integrity-checked, and vendored with its MIT license. The custom host adapter owns a mutable native tree and dispatches retained closures, with no DOM.
- Rust receives owned nodes and uses the existing Taffy/fontdue scene and GPUI painter. It does not evaluate Deka expressions or own hook state.
- `deka_native_ir` now shares utility parsing and element defaults with both execution paths, via DSC PR 308 commit `15b8ae547da6174616186f14a8facb9edd9b90b5`. No compiler library dependency is added to the runtime binary.
- Native polling services Deno timers and async completion without waiting for long-lived work to finish. Unmount runs React cleanup. Unsupported native elements, props, and utilities report errors.

## Evidence (Intel macOS, Rust 1.96.0)

- Normal DSC CLI compiles the imported DSX fixture, including compiler-generated `useState`, `useMemo`, and `useCallback` calls.
- `DEKA_DSC=/absolute/path/to/dsc cargo test --locked --release -p deka-native --features runtime -- --test-threads=1`: 8 integration tests pass (5 existing development-host tests, 3 new runtime tests).
- New tests cover independent component state, conditional unmount/remount, stale-handler rejection, changed text and colors, inherited glyph color, async handler suspension/completion, effect cleanup/error propagation, denied Rust filesystem operation, production permission-profile selection, hidden raw Deno API, and explicit unsupported-UI diagnostics.
- Layout oracle at a 560×300 viewport: first button `(16,16,96,64)`, second at `(124,16)`, toggle at `y=96`, third counter at `y=160`. The 12px row gap is not clickable. A 2× frame retains logical geometry and paints the updated purple fill with an 8px radius.
- Mutation proof: disabling the reconciler's text-commit operation leaves `Count: 0`; the compiled-component test fails. Restoring it makes the tests pass.
- `cargo test --locked --release -p deka_native_web`: 2 tests pass, including shared scene/viewport/styling and pointer/keyboard/reload behavior. This is a host-side preview regression test, not a new browser deployment.
- `cargo test --locked --release -p pool thrown_host_ops_return_error_envelopes`: the existing runtime host-operation regression passes using the extracted bootstrap.
- `cargo check --locked --workspace --all-targets`: passes.
- `cargo clippy --locked --workspace --all-targets --all-features`: passes; existing unrelated warnings, none in new/touched native code.
- `cargo build --locked --release -p cli -p deka-native --features deka-native/runtime,deka-native/runtime-shaders`: passes.
- File-size gate and whitespace checks pass. Bootstrap extraction takes the previous oversized worker file below its baseline threshold.
- Windowed command's `--exercise 3` prints `Count:  3 Count:  0 Toggle third counter Count:  0`. Native window opened and Sami confirmed it works. Screenshot after user interaction: [window.png](evidence/native-runtime-1177/window.png).

## Boundaries

This is a single-window desktop experiment, not a production runtime launch mode. The browser tour retains its restricted IR evaluator; browser React execution is a separate integration. The existing restricted `dev` path retains source reload; the new runtime command does not yet implement hot reload or production bundling. Layout support remains the documented utility/element subset. Alignment utilities, wrapping, scrolling, inputs, accessibility and pixel-perfect typography are not part of this slice. The full host's binary/memory cost is not the earlier small benchmark's result.

-codex
