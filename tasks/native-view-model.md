# Native view API implementation reference

The canonical design is [APS 73](https://github.com/dekaruntime/aps/issues/73).
It records Sami's note-08 decisions and visibly separates the proposed checked
API details. Keep design discussion and amendments there rather than copying
its API table into this repository. Implementation tracking: dekaruntime/deka#1293
for this reference, followed by the note-08 task list.

This is a design reference, not a claim that the view API is available.
Stable node/element handles, selectors, imperative writes, dependency tracking
and listeners remain implementation tasks. The proposed type names, selector
scope and keyed list contract remain for review in APS 73.

## Current boundary

`component::Component::render` resolves getters and builds a `ComponentFrame`.
`ui::UiSession` stores its presentation tree; reconciliation retains matching
positional structure, but the emitted path is not a durable node identity.
A whole render still reevaluates bindings and can overwrite an imperative edit.
The new shared tree store belongs above rendering, with both native and browser
adapters consuming the same snapshots. It must not move VM scheduling into the
renderer or text code. APS 7 and dekaruntime/deka#1288 provide the bounded
turn/ready/wake interface; host values and rooted callbacks provide the handle
and event bridge.

## Delivery and acceptance evidence

Follow note 08 in order: lasting tree/identity, binding patches, checked node
handles, reads, writes, listeners, then browser integration. Each implementation
has a dedicated issue/PR and preserves the corpus gate.

Prove these effects through the actual component/session path:

- A button finds a node, edits text/classes, appends and removes a child, and
  the visible tree changes on the next frame.
- Those edits survive an unrelated state change and reevaluation whose
  authored binding value did not change.
- Changing that binding's authored value replaces only its own property;
  unrelated nodes, handlers and identities survive.
- Unreachable detached node/listener cycles are collected even when a listener
  captured its own node handle; host callback rooting must not hide that cycle.
- Async listener work survives the handler's return and updates the tree on
  completion, without a blocking window turn or idle redraws.
- Native and browser adapters execute the same source/actions and produce
  matching tree snapshots/scenes. The browser does not supply HTML/CSS layout.

Implementation PRs include reverted-fix failures for those effects. No new
runtime test is added for this documentation reference: the behavior is not
implemented by a Markdown file.

-codex
