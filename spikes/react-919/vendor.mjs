// Import-once preparation. All foreign bytes are committed; no package manager.
import fs from 'node:fs';
import {createHash} from 'node:crypto';
import {spawnSync} from 'node:child_process';
import path from 'node:path';
import {pathToFileURL} from 'node:url';
process.chdir(import.meta.dirname);
const deka = path.resolve('.toolchain/bin/deka');
for (const [file, pin] of Object.entries(JSON.parse(fs.readFileSync('vendor-inputs/sources.json')))) {
  const actual = createHash('sha256').update(fs.readFileSync(`vendor-inputs/${file}`)).digest('hex');
  if (actual !== pin.sha256) throw Error(`Original vendor input hash mismatch: ${file}`);
}
fs.mkdirSync('.cache/prepared', {recursive:true});
function summon(name, source) {
  const file = path.resolve('.cache/prepared', `${name}.mjs`);
  fs.writeFileSync(file, source);
  const result = spawnSync(deka, ['summon', `url:${pathToFileURL(file)}`], {encoding:'utf8'});
  process.stdout.write(result.stdout); process.stderr.write(result.stderr);
  if(result.status !== 0) throw Error(`summon ${name} failed`);
}
const inputs = Object.fromEntries(['react','react-dom-server','ink'].map(name => [name, fs.readFileSync(`vendor-inputs/${name}.mjs`,'utf8')]));
const builtins = [...new Set(Object.values(inputs).flatMap(s => [...s.matchAll(/['"]node:([a-z_]+)['"]/g)].map(m=>m[1])))].sort();
fs.writeFileSync('host-builtins.json', JSON.stringify(builtins,null,2)+'\n');
for(const name of builtins) {
  const ns = await import(`node:${name}`);
  const keys = Object.keys(ns).filter(k=> k !== 'default' && /^[a-zA-Z_$][\w$]*$/.test(k));
  summon(`host-${name}`, `// Spike host capability bridge. Supplied before module loading.\nconst ns = globalThis.__react919Host[${JSON.stringify(name)}];\nexport default ns.default;\n` + keys.map(k=>`export const ${k} = ns.${k};`).join('\n')+'\n');
}
summon('devtools-disabled', 'export default {initialize(){throw new Error("DevTools are disabled in the React 919 spike")},connectToDevTools(){throw new Error("DevTools are disabled in the React 919 spike")}};\n');
for(const [name,input] of Object.entries(inputs)) {
  let source = input.replace(/(["'])node:([a-z_]+)\1/g, (_,q,n)=>`${q}@js/host-${n}${q}`)
    .replaceAll('"/react@19.1.1/node/react.mjs"','"@js/react"')
    .replaceAll('"react"','"@js/react"')
    .replaceAll('"/react-devtools-core@^6.1.2?target=node"','"@js/devtools-disabled"');
  summon(name, source);
}
