import {spawnSync} from 'node:child_process';
import {performance} from 'node:perf_hooks';
import os from 'node:os';
import fs from 'node:fs';
import zlib from 'node:zlib';
process.chdir(import.meta.dirname);
const samples = {};
for (const mode of ['ssr','ink']) {
  samples[mode] = [];
  for(let i=0;i<10;i++) {
    const start = performance.now();
    const result = spawnSync(process.execPath,['run.mjs',mode],{env:{...process.env,NODE_ENV:'production'},encoding:'utf8'});
    if(result.status !== 0) throw Error(result.stderr);
    samples[mode].push({...JSON.parse(result.stderr.trim()),processWallMs:performance.now()-start});
  }
}
const median = values => {const s = values.toSorted((a,b)=>a-b); return (s[4]+s[5])/2;};
const summary = Object.fromEntries(Object.entries(samples).map(([mode,rows])=>[mode,Object.fromEntries(Object.keys(rows[0]).filter(k=>k.endsWith('Ms')).map(k=>[k,{median:median(rows.map(r=>r[k])),min:Math.min(...rows.map(r=>r[k])),max:Math.max(...rows.map(r=>r[k]))}]))]));
const bytes = Object.fromEntries(['react','react-dom-server','ink'].map(name=>{const b=fs.readFileSync(`js_modules/${name}/index.mjs`); return [name,{raw:b.length,gzip:zlib.gzipSync(b).length}];}));
const report = {host:{platform:os.platform(),release:os.release(),arch:os.arch(),cpu:os.cpus()[0].model,node:process.version},method:'10 sequential fresh Node processes per renderer, warm OS file cache, NODE_ENV=production; imports include spike loader and host bridge; render includes first commit/output; wall includes process launch/shutdown. Not a Deka runtime or Electron benchmark.',bytes,summary,samples};
fs.writeFileSync('evidence/perf.json',JSON.stringify(report,null,2)+'\n');
console.log(JSON.stringify({host:report.host,bytes,summary},null,2));
