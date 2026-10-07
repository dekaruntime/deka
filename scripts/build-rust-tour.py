#!/usr/bin/env python3
"""Build the precompiled Rust tour and repo-derived source manifest (no release)."""
import argparse
import gzip
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent.parent

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--commit', help='Immutable commit of a source archive without .git')
    args = parser.parse_args()
    if (ROOT / '.git').exists():
        commit = subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
        if subprocess.check_output(['git','status','--porcelain','--untracked-files=no'],cwd=ROOT,text=True).strip():
            raise SystemExit('Build the tour from a clean committed source tree')
    else:
        commit = args.commit or ''
    if not re.fullmatch('[a-f0-9]{40}', commit):
        raise SystemExit('An immutable source commit is required')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if subprocess.check_output(['wasm-bindgen','--version'], text=True).strip() != 'wasm-bindgen 0.2.128':
        raise SystemExit('wasm-bindgen-cli 0.2.128 is required')
    # Ask cargo where it builds: CARGO_TARGET_DIR, .cargo/config or the default target/.
    target = Path(json.loads(subprocess.check_output(['cargo','+1.96.0','metadata','--format-version','1','--no-deps'],cwd=ROOT,text=True))['target_directory'])
    env = dict(os.environ)
    sysroot = subprocess.check_output(['rustc', '+1.96.0', '--print', 'sysroot'], text=True).strip()
    cargo_home = Path(env.get('CARGO_HOME', Path.home()/'.cargo')).resolve()
    env['RUSTFLAGS'] = ' '.join([env.get('RUSTFLAGS', ''),
        f'--remap-path-prefix={ROOT}=/deka',
        f'--remap-path-prefix={cargo_home}=/cargo',
        f'--remap-path-prefix={sysroot}=/rust-toolchain'])
    def cargo(*args):
        subprocess.run(['cargo','+1.96.0',*args], cwd=ROOT, env=env, check=True)
    tree = subprocess.check_output(['cargo','+1.96.0','tree','--locked','-p','deka-ui-tour','--target','wasm32-unknown-unknown','--edges','normal','--prefix','none','--format','{p}'],cwd=ROOT,text=True)
    crates = {line.split()[0] for line in tree.splitlines()}
    allowed = set((ROOT/'scripts/rust-tour-crates.txt').read_text().splitlines())
    unexpected = crates - allowed
    if unexpected:
        raise SystemExit('Unreviewed browser dependencies: '+', '.join(sorted(unexpected)))
    cargo('build','--locked','--profile','native','--target','wasm32-unknown-unknown','-p','deka-ui-tour')
    subprocess.run(['wasm-bindgen',str(target/'wasm32-unknown-unknown/native/deka_ui_tour.wasm'),'--target','web','--omit-default-module-path','--out-dir',str(out)],check=True)
    cargo('run','--locked','--release','-p','deka-ui-tour','--example','package_sources','--',str(out))
    cargo('build','--locked','--profile','native','--target','wasm32-unknown-unknown','-p','deka-ui-tour','--features','web-test')
    subprocess.run(['wasm-bindgen',str(target/'wasm32-unknown-unknown/native/deka_ui_tour.wasm'),'--target','web','--omit-default-module-path','--out-dir',str(out/'test-fixture')],check=True)
    cargo('build','--locked','--profile','native','--target','wasm32-unknown-unknown','-p','deka-ui-tour','--features','web-test','--example','web_input')
    subprocess.run(['wasm-bindgen',str(target/'wasm32-unknown-unknown/native/examples/web_input.wasm'),'--target','web','--omit-default-module-path','--out-dir',str(out/'input-fixture')],check=True)
    cargo('build','--locked','--profile','native','--target','wasm32-unknown-unknown','-p','deka-ui-tour','--features','web-test','--example','web_shared')
    subprocess.run(['wasm-bindgen',str(target/'wasm32-unknown-unknown/native/examples/web_shared.wasm'),'--target','web','--omit-default-module-path','--out-dir',str(out/'shared-fixture')],check=True)
    # wasm-bindgen copies its directly referenced module, not that module's
    # relative imports. Test-only wrappers share the exact production host.
    for fixture in ['test-fixture', 'input-fixture', 'shared-fixture']:
        for wrapper in (out/fixture).rglob('test-host.js'):
            shutil.copyfile(ROOT/'crates/deka_ui/web/host.js',wrapper.parent/'host.js')
    shutil.copyfile(ROOT/'crates/deka_native_ui/assets/OFL.txt',out/'font-OFL.txt')
    files={str(p.relative_to(out)):{'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted(out.rglob('*')) if p.is_file() and p.name!='manifest.json'}
    wasm=out/'deka_ui_tour_bg.wasm'
    for checkout_path in [str(ROOT), str(cargo_home), sysroot]:
        if checkout_path.encode() in wasm.read_bytes():
            raise SystemExit('Absolute build path leaked into wasm: '+checkout_path)
    manifest={'schemaVersion':1,'runtime':'deka-ui','commit':commit,'source':'https://github.com/dekaruntime/deka','wasmBindgen':'0.2.128','rustc':subprocess.check_output(['rustc','+1.96.0','--version'],text=True).strip(),'target':'wasm32-unknown-unknown','gzipBytes':len(gzip.compress(wasm.read_bytes(),compresslevel=9,mtime=0)),'cargoLockSha256':hashlib.sha256((ROOT/'Cargo.lock').read_bytes()).hexdigest(),'files':files}
    (out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
    print(f'Rust tour: {len(wasm.read_bytes())} bytes; {manifest["gzipBytes"]} gzip bytes; {commit}')
if __name__=='__main__':
    main()
