# Native boxes and text

The experimental native renderer lays out owned UI nodes. It does not create
HTML elements. The same Rust layout and text code runs in the desktop host and
WASM preview. The desktop component host supports normal DekaScript components
and computed class strings; the tour still uses the restricted single-component
native compiler with literal class alternatives and numeric state.

## Defaults

Every container uses Flexbox. `div` defaults to a column; `span`, `p` and `button`
default to a row. Children start at the leading edge, stretch in the cross axis,
grow by zero and shrink by one. Wrapping is off for boxes and on for text.
Wrapping rows pack lines from the start. Dimensions are border-box dimensions.
An automatic root width fills the viewport; root height is content-driven unless
specified. `h-full` requests the viewport height at the root. Nested fractional
sizes resolve against the parent's available inner size using Taffy rules.

Default padding, margins and gaps are zero. Containers have no fill and no
rounded corners; the window canvas behind them is white. Text inherits colour
and size, starting at dark `#1A1611`, 16 logical pixels and bundled Atkinson
Hyperlegible Regular. Buttons retain the experiment's 12px padding, 6px radius
and teal `#226C65` background. Device scale changes glyph resolution, not layout.

## Styles

Tokens apply in source order. Later tokens override the properties they set:
`p-4 px-2 pt-1` means top 4, right 8, bottom 16, left 8 logical pixels. Numeric
spacing values multiply by four. Unsupported or invalid tokens report an error.

| Purpose | Utilities |
| --- | --- |
| Direction and box wrapping | `flex`, `flex-row`, `flex-col`, `flex-wrap`, `flex-nowrap` |
| Cross-axis alignment | `items-start`, `items-center`, `items-end`, `items-stretch` |
| Per-child alignment | `self-auto`, `self-start`, `self-center`, `self-end`, `self-stretch` |
| Main-axis alignment | `justify-start`, `justify-center`, `justify-end`, `justify-between`, `justify-around`, `justify-evenly` |
| Flexible sizes | `grow`, `grow-0`, `shrink`, `shrink-0`, `flex-none` |
| Dimensions and limits | `w-*`, `h-*`, `min-w-*`, `min-h-*`, `max-w-*`, `max-h-*` |
| Padding | `p-*`, `px-*`, `py-*`, `pt-*`, `pr-*`, `pb-*`, `pl-*` |
| Margins | `m-*`, `mx-*`, `my-*`, `mt-*`, `mr-*`, `mb-*`, `ml-*` |
| Gaps | `gap-*`, `gap-x-*`, `gap-y-*` |
| Overflow | `overflow-visible`, `overflow-hidden` |
| Text wrapping | `whitespace-normal`, `whitespace-nowrap` (inherited) |
| Corners | `rounded-none`, `rounded`, `rounded-lg` |
| Text size | `text-sm`, `text-base`, `text-lg`, `text-xl`, `text-2xl` |
| Colours | `bg-[#RRGGBB]`, `text-[#RRGGBB]` |

Dimensions accept numeric spacing, `auto`, `full`, and fractions such as `1/2`.
Spacing accepts nonnegative numbers only; auto/negative margins are unsupported.
A flex item's automatic minimum can preserve its content width: use `min-w-0`
when you explicitly want it to shrink below that minimum. `overflow-hidden`
clips at a rectangular box; it does not create a scroll container or lower an
automatic minimum. Clipping affects paint and pointer targets together. Rounded
background corners do not imply rounded clipping or rounded hit targets.

## Text flow

Adjacent bare text fragments, including interpolated values, become one text
run. That run wraps using fontdue's word layout; explicit element boundaries
remain separate flex boxes. Nested spans do not yet form a rich inline paragraph.
Measurement and painting use the same logical-pixel text layout. No browser font
metrics or browser CSS are consulted. Fontdue is not a complex-script shaping
engine: shaping, bidi, fallback fonts, selectable/editable text, ellipsis and
full CSS whitespace semantics remain outside this slice.

## Try component-controlled alignment

`examples/native/runtime/layout.dsx` starts with a button at `(16,16)` in a
560×300 viewport. Clicking changes the root to centered alignment, moving the
192px-wide button to `x=184` and centering the group vertically. Clicking again
restores the original position. The adjacent text wraps within a 192px box.

Run from the Deka checkout after building the GPU runtime:

```sh
.target/release/deka-native runtime examples/native/runtime/layout.dsx \
  --project examples/native/runtime --compiler /absolute/path/to/dsc --component App
```

The tour's **Layout basics** example demonstrates the shared subset with literal
styles. Resize the preview, edit alignment/gaps, and click the counter.

## Boundaries

This adds box layout, text wrapping and rectangular clipping. It does not add
scrolling, Grid, absolute positioning, borders, editable
controls, accessibility bridges, event bubbling, complete pointer gestures or
incremental Rust tree updates. Existing pointer activation timing is still a
prototype difference between desktop and browser; this slice verifies shared
layout/paint/hit regions rather than claiming full input parity.

Presentation transitions are documented in [Native animation](native-animation.md).
