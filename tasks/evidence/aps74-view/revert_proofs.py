from pathlib import Path
import difflib, subprocess, sys
root=Path(__file__).resolve().parents[3]
runner=Path('/Volumes/Projects/codex/deka-ui-logs/run.py')
proofs=[
 ('view-revert-patching','crates/deka_ui/src/view.rs','context.changed(patch(node, value));','context.changed(false); drop(value);','--test','view','clicks_patch_only_the_bound_property_of_the_retained_node'),
 ('view-revert-event-batch','crates/deka_ui/src/view.rs','self.scope.batch(|| callback.borrow_mut()(event));','self.scope.run(|| callback.borrow_mut()(event));','--test','view','an_event_batches_multiple_writes_into_one_final_binding_pass'),
 ('view-revert-disposal','crates/deka_ui/src/view.rs','for effect in self.effects.drain(..) {\n            effect.dispose();\n        }','for _effect in self.effects.drain(..) {}','--test','view','removed_bindings_stop_observing_and_event_captures_are_released'),
 ('view-revert-authored-ownership','crates/deka_native_ir/src/tree.rs','if self.authored_text == text {','if self.text == text {','--lib','','view::tests::imperative_edits_survive_unchanged_authored_values_and_other_property_patches'),
 ('view-revert-class-kind','crates/deka_native_ir/src/tree.rs','text: matches!(&record.kind, Kind::Text).then(String::new),','text: record.text.clone(),','--lib','','view::tests::imperative_edits_survive_unchanged_authored_values_and_other_property_patches'),
]
for name,relative,old,new,mode,target,test in proofs:
 path=root/relative; original=path.read_text();assert original.count(old)==1,(name,original.count(old))
 changed=original.replace(old,new)
 (Path(__file__).parent/(name+'.patch')).write_text(''.join(difflib.unified_diff(original.splitlines(True),changed.splitlines(True),fromfile='a/'+relative,tofile='b/'+relative)))
 try:
  path.write_text(changed)
  args=['cargo','test','--release','--locked','-p','deka-ui','--no-default-features',mode]
  if target:args.append(target)
  args.extend([test,'--','--exact'])
  code=subprocess.call([sys.executable,str(runner),name,*args],cwd=root)
  log=(runner.parent/(name+'.log')).read_text()
  assert code==101 and 'assertion' in log and 'FAILED' in log and 'error[E' not in log,(name,code)
 finally:path.write_text(original)
print('Five literal source removals fail behavior assertions with 101; all original source restored.')
