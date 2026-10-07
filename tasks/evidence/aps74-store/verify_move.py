from pathlib import Path
import subprocess
import re
root = Path(__file__).resolve().parents[3]
def before(path):
    return subprocess.check_output(["git", "show", "08511f1b:" + path], cwd=root, text=True)
def current(path):
    return (root / path).read_text()
for file in ["tree", "selectors"]:
    path = f"crates/deka_vm/src/component/{file}.rs"
    marker = "#[cfg(test)]\nmod tests"
    assert before(path)[before(path).index(marker):] == current(path)[current(path).index(marker):], file
def normalize(code):
    code = re.sub(r"pub(?:\([^)]*\))?\s+", "", code)
    code = re.sub(r'#\[doc\(hidden\)\]\s*', '', code)
    code = code.replace('#[cfg(any(test, feature = "v8-control"))]', '')
    return re.sub(r",(?=\))", "", re.sub(r"\s+", "", code))
for file in ["tree", "selectors"]:
    old = before(f"crates/deka_vm/src/component/{file}.rs").split("#[cfg(test)]\nmod tests")[0]
    new = current(f"crates/deka_native_ir/src/{file}.rs")
    if file == "tree":
        old = old.replace('use crate::{Result, ui::WireNode};\nuse deka_native_ui::{Node, Style};',
                          'use crate::{Node, Style, WireNode};\ntype Result<T> = std::result::Result<T, String>;')
    assert normalize(old) == normalize(new), f"{file}: implementation changed"
old = before("crates/deka_vm/src/ui.rs")
new = current("crates/deka_native_ir/src/wire.rs")
old = old[old.index("impl WireNode"):old.index("/// Shared event-to-tree adapter")]
new = new[new.index("impl WireNode"):]
old = old.replace("deka_native_ui::Style", "crate::Style").replace("deka_native_ir::", "crate::")
assert normalize(old) == normalize(new), "wire preparation changed"
for path in ["crates/deka_vm/tests", "crates/deka_fmt/tests/fixtures/tour", "tests/corpus-passing.txt"]:
    assert not subprocess.check_output(["git", "diff", "08511f1b", "--", path], cwd=root), path
assert "fn retain(" not in current("crates/deka_vm/src/component/tree.rs")
assert "fn retain(" in current("crates/deka_native_ir/src/tree.rs")
assert "fn style(" not in current("crates/deka_vm/src/ui.rs")
print("Original VM test bodies, all integration tests, tour fixtures and corpus list unchanged; one store/style preparation implementation.")
