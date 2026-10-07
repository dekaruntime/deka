import subprocess, sys
from pathlib import Path
runner='/Volumes/Projects/codex/deka-ui-logs/run.py'
steps=[
('view-check',['cargo','check','--release','--locked','--workspace','--all-targets']),
('view-clippy',['cargo','clippy','--release','--locked','--workspace','--all-targets','--all-features']),
('view-strict',['cargo','clippy','--release','--locked','-p','deka-ui','-p','deka_native_ir','--all-targets','--all-features','--no-deps','--','-D','warnings']),
('view-tests',['cargo','test','--release','--locked','-p','deka-ui','--all-features']),
('view-native-reference',['./run.sh']),
('view-fmt',['cargo','fmt','-p','deka-ui','-p','deka_native_ir','--check']),
('view-counter',['cargo','run','--release','--locked','-p','deka-ui','--example','counter','--','--exercise','3']),
('view-window',['cargo','run','--release','--locked','-p','deka-ui','--example','counter','--','--smoke']),
('view-package-check',['cargo','check','--release','--locked','--manifest-path','tools/deka-package/Cargo.toml','--all-targets']),
('view-package-vm',['cargo','build','--release','--locked','-p','deka_vm','--no-default-features','--features','gpu','--bin','dvm-ui']),
('view-package-build',['cargo','build','--release','--locked','--manifest-path','tools/deka-package/Cargo.toml']),
('view-package-relocated',['python3','tools/deka-package/test_bundle.py','.target/release/dvm-package','.target/release/dvm-ui','crates/deka_vm/examples/packaged-counter/deka.json','/Volumes/Projects/codex/.tmp-deka/aps74-view-package-tests']),
]
for name,command in steps:
 code=subprocess.call([sys.executable,runner,name,*command])
 if code:sys.exit(code)
 if name=='view-counter': assert Path(runner).with_name(name+'.log').read_text().rstrip().endswith('Count: 3 Add one')
 if name=='view-window': assert 'Presented Rust counter frame 1' in Path(runner).with_name(name+'.log').read_text()
