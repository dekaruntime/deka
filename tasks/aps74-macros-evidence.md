# APS 74 macro evidence

Step 2 stacks on View PR #1413. No VM source or original tour fixture changes.

rstml 0.13.1 was evaluated against deka_syntax with the actual same token input.
rstml parses the event as a Rust closure and preserves its original line 3;
stringifying for the DekaScript parser collapses the markup to line 1. The latter
accepts this input, but provides its own language AST rather than Rust expressions.
The token/span route was selected. Compile-fail fixtures verify unknown
component, missing required prop and wrong prop type on the user's markup lines.
They run release-only nested cargo checks serially, not trybuild's debug builds.

Components remain ordinary functions with generated typed props. Imported
functions alone suffice; markup infers the props builder from their signature.
Missing props have no runtime fallback. Defaults and single-use Children are
covered. Native tags/attributes/literal utilities validate against the existing
catalog. Quoted interpolation registers live bindings and preserves native text
fragments; static and live expression attributes, class toggles, typed events,
Options and iterators all run through UiApp and the shared retained store.

Three literal source removals each fail an assertion with exit 101: freeze
quoted signal text, discard an explicit prop default, and replace the component
tag's diagnostic span with the macro invocation span. The third failure contains
the expected fixture compiler diagnostic as assertion data; the root test itself
compiles and fails on the wrong primary source line. Source is restored in finally.

Final results: 33 tests pass, exit 0. The macro counter after three events
prints `Count:  3 Add one`, exit 0. The exercise helper inserts a separator
between text fragments; rendered text remains `Count: 3`, as the host test asserts.

Final workspace check/clippy, strict touched-crate clippy, tests, format and
macro-counter exercise logs and exit files are committed in
`tasks/evidence/aps74-macros/`. The own docs page is `docs/rust-ui/markup.mdx`.
The unchanged 27-lesson scene comparison is the next PR.

-codex
