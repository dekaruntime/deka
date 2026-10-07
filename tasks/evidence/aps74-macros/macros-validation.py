import subprocess,sys
from pathlib import Path
runner='/Volumes/Projects/codex/deka-ui-logs/run.py'
steps=[
('macros-check',['cargo','check','--release','--locked','--workspace','--all-targets']),
('macros-clippy',['cargo','clippy','--release','--locked','--workspace','--all-targets','--all-features']),
('macros-strict',['cargo','clippy','--release','--locked','-p','deka-ui','-p','deka-ui-macros','--all-targets','--all-features','--no-deps','--','-D','warnings']),
('macros-tests',['cargo','test','--release','--locked','-p','deka-ui','-p','deka-ui-macros','--all-features']),
('macros-fmt',['cargo','fmt','-p','deka-ui','-p','deka-ui-macros','--check']),
('macros-counter',['cargo','run','--release','--locked','-p','deka-ui','--example','counter_macro','--','--exercise','3']),
]
for name,command in steps:
 code=subprocess.call([sys.executable,runner,name,*command])
 if code:sys.exit(code)
 if name=='macros-counter':assert Path(runner).with_name(name+'.log').read_text().rstrip().endswith('Count:  3 Add one')
