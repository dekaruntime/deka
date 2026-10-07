from pathlib import Path
import difflib, subprocess, sys
root=Path(__file__).resolve().parents[3]
runner=Path('/Volumes/Projects/codex/deka-ui-logs/run.py')
path=root/'crates/deka_ui/examples/tour/counter.rs'
original=path.read_text()
assert original.count('count += 1')==1
changed=original.replace('count += 1','count += 2')
(Path(__file__).parent/'parity-revert-counter.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True),changed.splitlines(True),fromfile='a/crates/deka_ui/examples/tour/counter.rs',tofile='b/crates/deka_ui/examples/tour/counter.rs')))
try:
 path.write_text(changed)
 code=subprocess.call([sys.executable,str(runner),'parity-revert-counter','cargo','test','--release','--locked','-p','deka-ui','--no-default-features','--test','tour_parity','counter','--','--exact','--nocapture'],cwd=root)
 log=(runner.parent/'parity-revert-counter.log').read_text()
 assert code==101 and 'test result: FAILED' in log and 'time=100ms: scene.' in log and 'error: could not compile' not in log,code
finally:path.write_text(original)
print('Actual Rust counter increment changed to 2: complete-scene gate fails after the first click with 101; original source restored.')
