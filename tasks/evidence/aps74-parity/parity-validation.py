import subprocess,sys
from pathlib import Path
runner='/Volumes/Projects/codex/deka-ui-logs/run.py'
steps=[
('parity-check',['cargo','check','--release','--locked','--workspace','--all-targets']),
('parity-clippy',['cargo','clippy','--release','--locked','--workspace','--all-targets','--all-features']),
('parity-strict',['cargo','clippy','--release','--locked','-p','deka-ui','-p','deka-ui-macros','--all-targets','--all-features','--no-deps','--','-D','warnings']),
('parity-gate',['cargo','test','--release','--locked','-p','deka-ui','--no-default-features','--test','tour_parity','--','--nocapture']),
('parity-rust-ui',['./scripts/test-rust-ui.sh']),
('parity-native-reference',['./run.sh']),
('parity-fmt',['cargo','fmt','-p','deka-ui','-p','deka-ui-macros','--check']),
('parity-counter',['cargo','run','--release','--locked','-p','deka-ui','--example','tour-counter','--','--exercise','3']),
('parity-production-graph',['cargo','tree','--locked','-p','deka-ui','-e','normal','--prefix','none']),
('parity-reference-source',['git','diff','--exit-code','codex/shared-store-aps74','--','crates/deka_vm','tests/corpus-passing.txt','crates/deka_fmt/tests/fixtures/tour','run.sh']),
]
for name,command in steps:
 code=subprocess.call([sys.executable,runner,name,*command])
 if code:sys.exit(code)
 if name=='parity-counter':assert Path(runner).with_name(name+'.log').read_text().rstrip().endswith('Count:  3')
 if name=='parity-production-graph':
  graph=Path(runner).with_name(name+'.log').read_text()
  assert 'deka_vm ' not in graph and 'deka_syntax ' not in graph
