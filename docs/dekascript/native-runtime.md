# Native component runtime experiment

The `deka-native runtime` command runs normal DSC-compiled components in a persistent Deka V8 isolate and paints their output with the shared Rust renderer. It uses the existing module loader, host bootstrap, host extensions and project security policy. React 19.1.1 and a pinned React reconciler own component state, memoization, event closures and effect cleanup. The native window contains no WebView or DOM.

Build the experimental host with `cargo build --locked --release -p deka-native --features runtime,runtime-shaders` on macOS without the Metal compiler, or `--features runtime,gpu` with the platform build prerequisites available. Pass the normal DSC binary, not `dsc-native`:

```sh
.target/release/deka-native runtime examples/native/runtime/app.dsx \
  --project examples/native/runtime \
  --compiler /absolute/path/to/dsc \
  --component App
```

The example has two independent counters and a third that can be removed and remounted. A click changes the count and its background color; remounting resets that counter's state. Add `--exercise 3` to invoke the first mounted handler three times without a display.

The UI host polls the existing Deno event loop without waiting for long-lived timers to finish. Timers use deno_core's timer queue. An asynchronous handler can update state after its promise completes. Shutdown unmounts the React root and runs effect cleanup.

## Current rendering surface

Elements: `div`, `span`, `p`, `button`. Supported props: `children`, `className`, `onClick`, `id`, `ref` and the compiler's `data-deka-id` metadata. Refs expose an opaque native identifier, not DOM methods. Clicks call the selected element's handler without a browser event or bubbling.

The compiler and live host share native element defaults and utility parsing: column/row flow, uniform padding, gap, fixed width/height, six-digit hex backgrounds and text colors, text sizes, and rounded corners. These remain a small explicit subset. Unsupported elements, props and classes produce diagnostics; inline CSS objects, general Tailwind, alignment utilities, wrapping, scrolling, input controls, accessibility and DOM APIs are not implemented.

Rust owns layout, glyph rasterization, paint commands and hit testing. The same scene implementation is used by the existing browser WASM/WebGL preview. This new command currently targets the desktop host; the tour still runs its earlier restricted IR evaluator. Its browser execution path has not been switched to React.

The earlier `dev` command retains its restricted-IR source watching and state-preserving reload. This new runtime command does not yet implement hot reload or production bundling. It links the full host dependencies and must not be compared with the small engine benchmark binaries as an application-size result. V8 remains the runtime baseline; the QuickJS experiment is separate.
