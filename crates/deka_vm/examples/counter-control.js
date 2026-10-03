// Matched V8 control for counter.dsx; deliberately no React or full Deka services.
let count = 0;
const text = value => ({text: String(value)});
const node = (tag, classes, children, handler = null) => ({tag, classes, children, handler});
function click(handler) {
    if (handler === 0) count += 1;
    else if (handler === 1) count = 0;
    else throw new Error("unknown handler");
}
function snapshot() {
    return node("view", "p-6 gap-4 bg-[#F3EFE3] text-[#1A1611]", [
        node("p", "text-2xl", [text("Deka native VM")]),
        node("p", "", [text("DekaScript markup, state and handlers. Rust draws the window.")]),
        node("p", "text-2xl", [text("Count: "), text(count)]),
        node("div", "flex-row gap-3", [
            node("button", "", [text("Add one")], 0),
            node("button", "", [text("Reset")], 1),
        ]),
    ]);
}
