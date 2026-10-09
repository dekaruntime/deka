#!/usr/bin/env python3
"""Literal deletion of the new integration/gates, with restored-source checks.
Run alone: this intentionally edits the implementation and restores it in finally.
"""
import os
from pathlib import Path
import re
import subprocess
ROOT=Path(__file__).resolve().parents[2]
LOGS=ROOT/'tasks/evidence/ui-hot-reload'
LOGS.mkdir(parents=True,exist_ok=True)


def run(name,command,expected):
    with (LOGS/(name+'.log')).open('w') as log:
        result=subprocess.run(command,cwd=ROOT,stdout=log,stderr=subprocess.STDOUT)
    assert (result.returncode==0)==expected,f'{name}: unexpected exit {result.returncode}'
    if not expected:
        output=(LOGS/(name+'.log')).read_text()
        assert ('AssertionError' in output or 'assertion failed' in output) and 'could not compile' not in output, f'{name}: failure was not the behavioral assertion'
    print(f'{name}: exit={result.returncode} ({"restored PASS" if expected else "literal revert FAIL"})',flush=True)


def remove_poll(source):
    start=source.index('    fn poll_reload(&mut self) -> deka_native_ui::Reload {')
    # Revert the Application implementation to its previous default Unchanged.
    start=source.rfind('    #[cfg',0,start)
    end=source.index('    fn run_turn(',start)
    return source[:start]+source[end:]


view=ROOT/'crates/deka_ui/src/view.rs'
original=view.read_text()
try:
    view.write_text(remove_poll(original))
    for case in ['markup','malformed']:
        run('revert-'+case,['python3','scripts/rust-ui-hot-reload/check.py','--case',case],False)
finally:
    view.write_text(original)
for case in ['markup','malformed']:
    run('restored-'+case,['python3','scripts/rust-ui-hot-reload/check.py','--case',case],True)

supervisor=ROOT/'crates/deka_ui_dev/src/main.rs'
original=supervisor.read_text()
try:
    # Delete child replacement from the new restart path; Cargo may still build
    # successfully, but the real PID/handler-effect assertion must fail.
    mutated=original.replace('                        child.stop()?;\n                        child = Running::new(&binary, &app_args)?;', '                        let _ = &binary;')
    assert mutated!=original
    supervisor.write_text(mutated)
    run('revert-supervisor-build',['cargo','build','--locked','-p','deka-ui-dev'],True)
    run('revert-restart',['python3','scripts/rust-ui-hot-reload/check.py','--case','restart','--restart-timeout','10'],False)
finally:
    supervisor.write_text(original)
run('restored-supervisor-build',['cargo','build','--locked','-p','deka-ui-dev'],True)
run('restored-restart',['python3','scripts/rust-ui-hot-reload/check.py','--case','restart'],True)

runtime=ROOT/'crates/deka_ui/src/hot_reload.rs'
original=runtime.read_text()
command=['cargo','test','--locked','-p','deka-ui','--no-default-features','--features','hot-reload,tour,web','--lib','hot_reload::tests::builder_attribute_overrides_rebuild_without_partial_patches']
try:
    guard='        if self.decorated {\n            return Err("builder decorations outside view! require rebuilding".into());\n        }\n'
    assert guard in original
    runtime.write_text(original.replace(guard,''))
    run('revert-builder-overrides',command,False)
finally:
    runtime.write_text(original)
run('restored-builder-overrides',command,True)

files=[ROOT/path for path in ['crates/deka_ui/src/lib.rs','crates/deka_ui/src/view.rs','crates/deka_native_ir/src/tree.rs','crates/deka_ui_macros/src/lib.rs']]
originals={path:path.read_text() for path in files}
try:
    # Revert the new debug-only application boundary in both macro and runtime.
    for path,source in originals.items():
        source=source.replace('debug_assertions,','').replace('debug_assertions, ','')
        source=source.replace('any(not(debug_assertions), target_arch="wasm32")','target_arch="wasm32"')
        path.write_text(source)
    run('revert-release',['python3','scripts/rust-ui-hot-reload/release.py'],False)
finally:
    for path,source in originals.items():path.write_text(source)
run('restored-release',['python3','scripts/rust-ui-hot-reload/release.py'],True)
