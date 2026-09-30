# Native presentation transitions

React commits target styles. Rust retains current styles by node ID and advances
transitions using the host's monotonic clock. Component code does not run once
per animation frame. The browser uses requestAnimationFrame; GPUI requests its
next frame only while a transition is active. The existing native runtime poll
for asynchronous JS work remains separate.

```tsx
export fn App() ReactNode {
    const [large, setLarge] = useState(false);
    return (
        <button className={large
            ? "w-64 h-20 transition-size duration-600 ease-in-out"
            : "w-40 h-12 transition-size duration-600 ease-in-out"}
            onClick={fn() { setLarge(!large); }}>Click to resize</button>
    );
}
```

## Contract

- `transition-opacity`: `opacity-0` through `opacity-100`.
- `transition-transform`: `translate-x-N`, `translate-y-N`, including negative
  prefixes. Numeric spacing uses four logical pixels per unit.
- `transition-size`: width/height between two pixel values or two percentages.
  Auto-to-fixed changes snap. Layout runs at each sample, moving neighbours.
- `transition-colors`: two explicit text/background colours, interpolated in
  sRGB channel values. Changes involving an inherited/absent colour snap.
- `transition-all`: all the properties above. `transition-none`: immediate.
- `duration-N`: milliseconds, 0–4096; default 200. Zero snaps.
- `ease-linear`, `ease-out` (default cubic), `ease-in-out` (smoothstep).

First mount snaps to its initial style. A later target change starts a transition
from the currently displayed value. Interruption samples the old transition
before changing direction. Removing a node removes its animation state on the
next frame. React keys preserve host identity across reordering; remounts get
fresh identity. Keep an element mounted to fade it out; deferred unmount and
enter/exit lifecycles are not implemented.

Opacity multiplies through descendants and is applied per paint primitive. It
is not isolated offscreen group compositing: overlapping children/backgrounds
can differ from CSS group opacity while translucent. Translation moves painting,
clips and hit regions together without moving neighbouring layout boxes. Fully
transparent or fully clipped controls have no pointer/keyboard target; controls
remain interactive while partially visible.

Browser reduced-motion preferences snap to the target, including when the
preference changes during a transition. Native hosts accept `--reduced-motion`;
OS preference detection is not yet wired. There is no environment-variable
configuration. The portable `render_at` / WASM `frame_at` API accepts explicit
milliseconds and a reduced-motion boolean for deterministic tests. Invalid or
backwards time does not rewind motion.

## Tour and native demos

The tour's Modal fade, Sliding menu, Growing button and Toast notification
examples use the same Rust animator. Their restricted compiler accepts numeric
state comparisons such as `open == 1` and two literal class alternatives. Normal
DekaScript/React components can compute their class strings normally.

These examples demonstrate presentation. They do not implement modal focus
trapping, overlay stacking, outside-click dismissal or notification timers.
Successful tour compilation preserves compatible application state but clears
animation history because edited template paths may identify different nodes.
A compilation error keeps the live app and its presentation intact.

The native React example is `examples/native/runtime/animation.dsx`:

```sh
.target/release/deka-native runtime examples/native/runtime/animation.dsx \
  --project examples/native/runtime --compiler /absolute/path/to/dsc --component App
```

Springs, keyframes, scale/rotation, layout-position transitions, layout-property
interpolation beyond width/height, and GPU group compositing remain future work.
