from pathlib import Path
import difflib, subprocess, sys
root=Path(__file__).resolve().parents[3]
runner=Path('/Volumes/Projects/codex/deka-ui-logs/run.py')
proofs=[
 ('macros-revert-live-text','::deka_ui::View::interpolate(&#name)','::deka_ui::View::text(format!("{}", #name))','-p','deka-ui','--test','macros','imported_component_props_defaults_children_and_interpolation_run_on_the_real_host'),
 ('macros-revert-defaults','let value: Expr = meta.value()?.parse()?;\n                    quote!(#value)','let _value: Expr = meta.value()?.parse()?;\n                    quote!(::core::default::Default::default())','-p','deka-ui','--test','macros','default_props_support_launch_style_functions_and_root_expression_fragments'),
 ('macros-revert-markup-spans','quote_spanned!(name.span()=> #name(::deka_ui::props_for(#name) #(#setters)* #children .build()))','quote_spanned!(Span::call_site()=> #name(::deka_ui::props_for(#name) #(#setters)* #children .build()))','-p','deka-ui-macros','--test','diagnostics','errors_point_at_the_users_markup_lines'),
]
path=root/'crates/deka_ui_macros/src/lib.rs'
for name,old,new,*test in proofs:
 original=path.read_text();assert original.count(old)==1,(name,original.count(old));changed=original.replace(old,new)
 (Path(__file__).parent/(name+'.patch')).write_text(''.join(difflib.unified_diff(original.splitlines(True),changed.splitlines(True),fromfile='a/crates/deka_ui_macros/src/lib.rs',tofile='b/crates/deka_ui_macros/src/lib.rs')))
 try:
  path.write_text(changed)
  code=subprocess.call([sys.executable,str(runner),name,'cargo','test','--release','--locked','--no-default-features',*test,'--','--exact'],cwd=root)
  log=(runner.parent/(name+'.log')).read_text()
  assert code==101 and 'test result: FAILED' in log and 'error: could not compile' not in log and ('assertion' in log or 'expected primary error' in log),(name,code)
 finally:path.write_text(original)
print('Three literal removals fail behavior/diagnostic assertions with 101; original source restored.')
