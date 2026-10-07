import os, subprocess, sys
from pathlib import Path
root=Path('/Volumes/Projects/codex/deka')
out=Path('/Volumes/Projects/codex/deka-ui-logs')
env=os.environ.copy(); env.update(TMPDIR='/Volumes/Projects/codex/.tmp-deka',CARGO_TARGET_DIR=str(root/'.target'))
name=sys.argv[1]; command=sys.argv[2:]
if (out/(name+'.log')).exists():
 index=1
 while (out/(name+f'.previous-{index}.log')).exists(): index+=1
 for suffix in ['.log','.exit']:
  path=out/(name+suffix)
  if path.exists(): path.rename(out/(name+f'.previous-{index}'+suffix))
print(name, command, flush=True)
with (out/(name+'.log')).open('w') as log:
 log.write('COMMAND: '+repr(command)+'\n');log.flush()
 result=subprocess.run(command,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
(out/(name+'.exit')).write_text(str(result.returncode)+'\n')
print(name,'EXIT',result.returncode,flush=True)
sys.exit(result.returncode)
