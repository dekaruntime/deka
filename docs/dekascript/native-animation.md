# Native presentation motion

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
  prefixes, `scale-N` (percent, 1–400), and `rotate-N` (degrees, negative prefix supported). Numeric translation spacing uses four logical pixels per unit.
- `transition-size`: width/height between two pixel values or two percentages.
  Auto-to-fixed changes snap. Layout runs at each sample, moving neighbours.
- `transition-colors`: two explicit text/background colours, interpolated in
  sRGB channel values. Changes involving an inherited/absent colour snap.
- `transition-all`: all the properties above. `transition-none`: immediate.
- `duration-N`: milliseconds, 0–4096; default 200. Zero snaps.
- `ease-linear`, `ease-out` (default cubic), `ease-in-out` (smoothstep).

First mount snaps unless an `enter-*` effect is requested. A later target change starts a transition
from the currently displayed value. Interruption samples the old transition
before changing direction. Removing a node normally removes its animation state on the next frame. React keys preserve host identity across reordering; remounts get
fresh identity. Use `exit-*` to retain an inert visual copy after removal. React unmounts immediately; effects and handlers are not retained. The visual copy occupies its old layout slot until the exit completes. Removing a whole subtree uses the removed parent's exit effect.

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

## Additional motion families

- `spring` selects an analytic unit-mass spring for transitions, presence and layout movement. Default stiffness 180 and damping 20; `spring-stiffness-N` (1–1000), `spring-damping-N` (1–100) tune it. `spring-none` returns to timed motion. Springs settle by an envelope threshold, capped at ten seconds. `duration-0` disables motion. Retargeting preserves position, but resets velocity in this first implementation.
- `animate-pulse`, `animate-bounce`, `animate-shake`, `animate-spin` are preset keyframes. `animate-none` cancels them. `duration-N` is one cycle; `repeat-N` counts cycles, `repeat-infinite` repeats until removed or cancelled, and `alternate` reverses every other cycle. Finite animations retain their last sampled value.
- Custom `frames-x-[0:0,50:80,100:0]`, `frames-y-[...]`, `frames-opacity-[0:0,100:1]`, `frames-scale-[0:1,50:1.2,100:1]`, and `frames-rotate-[0:0,100:360]` may be combined. Points are percentages, strictly increasing from 0 to 100, 2–32 points per property. Keyframe segments are linear. Translation/rotation add to the base style; scale/opacity multiply it.
- `enter-fade`, `enter-slide`, `enter-scale` and corresponding `exit-*` control presence; `enter-none` / `exit-none` disable it. Slide adds 24 logical pixels vertically; scale starts/ends at 85%; all three fade. Tour conditional children use `{open == 1 ? <div ... /> : None}`. Full React components may use normal conditional rendering.
- `transition-layout` moves an element from its previously displayed layout position to its new one after alignment, reordering or resize changes. Its ID must remain stable; React keys preserve identity. It uses the timing/spring settings on that element. `layout-none` disables it. Width/height interpolation remains `transition-size`.
- `delay-N` delays a track in milliseconds (0–10000). `stagger-N` on a parent spaces immediate children's starts (0–2000ms each). Delays also support simple sequences: give successive elements delay 0, 200, 400. This is declarative scheduling, not a general timeline API or completion callback system.

All motion runs in Rust. Reduced motion snaps transitions/layout, skips keyframes and entrances, and removes exits immediately. Infinite keyframes continue requesting frames until cancelled, removed or reduced motion is enabled.

Scale and rotation happen around each box's centre, compose through descendants, and transform clips and hit regions. Transforms do not reserve extra layout space. Scale/translation use GPU quads. Rotation currently uses shared CPU rasterization into textures, bounded to about one megapixel per primitive; large rotations may look softer. This guarantees both adapters use the same geometry but is not the final high-throughput GPU implementation. Focus outlines currently use the transformed bounding rectangle.

Group compositing, spring velocity continuity, arbitrary timeline orchestration, layout property interpolation beyond width/height, and automatic layout movement inside scaled/rotated ancestors need further work. Motion is presentation; accessibility semantics for dialogs and menus are separate.
