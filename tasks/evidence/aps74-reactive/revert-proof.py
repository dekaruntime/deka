import difflib, os, subprocess
from pathlib import Path
source = Path('/Volumes/Projects/codex/deka/crates/deka_ui/src/reactive.rs')
logs = Path('/Volumes/Projects/codex/deka-ui-logs')
original = source.read_text()
notifications = '''        self.dirty
            .borrow_mut()
            .extend(self.subscribers.borrow()[slot].iter().copied());'''
batch = '''            self.0.depth.set(self.0.depth.get() + 1);
            let _batch = Batch(self.0.clone());'''
tracking_start = original.index('        if let Some(reads) = core.reads.borrow_mut().last_mut()')
tracking_end = original.index('        self.value(&core)', tracking_start)
priority_start = original.index('                dirty\n                    .iter()')
priority_end = original.index('\n            };', priority_start)
removal = '''            for old in observer.dependencies.difference(&dependencies) {
                subscribers[*old].remove(&self.slot);
            }'''
proofs = [
 ('notifications', original.replace(notifications, ''), 'dependencies_follow_branches_and_unobserved_signals_do_no_work'),
 ('read-tracking', original[:tracking_start] + original[tracking_end:], 'copy_handles_events_arithmetic_toggle_and_display_are_reactive'),
 ('batching', original.replace(batch, ''), 'copy_handles_events_arithmetic_toggle_and_display_are_reactive'),
 ('old-dependencies', original.replace(removal, ''), 'dependencies_follow_branches_and_unobserved_signals_do_no_work'),
 ('derived-priority', original[:priority_start] + '                dirty.first().copied()' + original[priority_end:], 'effects_registered_before_a_later_derived_value_still_see_settled_state'),
 ('derived-equality', original.replace('if changed {', 'if true {'), 'derived_unchanged_values_do_not_notify_downstream_effects'),
]
env = os.environ.copy()
env['TMPDIR'] = '/Volumes/Projects/codex/.tmp-deka'
env['CARGO_TARGET_DIR'] = '/Volumes/Projects/codex/deka/.target'
try:
 for name, reverted, test in proofs:
  assert reverted != original, name
  source.write_text(reverted)
  (logs / f'revert-{name}.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True), reverted.splitlines(True), fromfile='a/crates/deka_ui/src/reactive.rs', tofile='b/crates/deka_ui/src/reactive.rs')))
  command = ['cargo', 'test', '--release', '--locked', '-p', 'deka-ui', '--test', 'reactive', test, '--', '--exact']
  with (logs / f'revert-{name}.log').open('w') as log:
   result = subprocess.run(command, cwd='/Volumes/Projects/codex/deka', env=env, stdout=log, stderr=subprocess.STDOUT)
  (logs / f'revert-{name}.exit').write_text(f'{result.returncode}\n')
  output = (logs / f'revert-{name}.log').read_text()
  print(f'{name}: exit {result.returncode}, assertion failed: {"assertion" in output}', flush=True)
  assert result.returncode == 101 and 'FAILED' in output and 'assertion' in output, output
  source.write_text(original)
finally:
 source.write_text(original)
