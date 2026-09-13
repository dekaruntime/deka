import { execFile } from 'node:child_process';
import { basename } from 'node:path';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const compilers = new Set(['cargo', 'rustc', 'clang', 'clang++', 'cc', 'cc1', 'ld', 'dsc']);
export const contention = { supported: ['darwin', 'linux'].includes(process.platform), pollMs: 500, waits: [], discarded: [] };

async function externalCompilers() {
  if (!contention.supported) return [];
  const output = await new Promise((resolve, reject) => execFile('ps', ['-axo', 'pid=,ppid=,comm='], (err, stdout) => err ? reject(err) : resolve(stdout)));
  const processes = output.trim().split('\n').map(line => {
    const m = line.trim().match(/^(\d+)\s+(\d+)\s+(.+)$/);
    return m && { pid: Number(m[1]), parent: Number(m[2]), executable: m[3] };
  }).filter(Boolean);
  const parents = new Map(processes.map(p => [p.pid, p.parent]));
  const ancestors = new Set();
  for (let p = process.pid; p && !ancestors.has(p); p = parents.get(p)) ancestors.add(p);
  function ours(pid) {
    const seen = new Set();
    while (pid && !seen.has(pid)) {
      if (pid === process.pid) return true;
      seen.add(pid); pid = parents.get(pid);
    }
    return false;
  }
  return processes.filter(p => compilers.has(basename(p.executable)) && !ancestors.has(p.pid) && !ours(p.pid));
}

// Guard boundaries plus periodic samples. This detects compiler contention,
// not all desktop activity or a hard OS reservation. Never pause other work.
export async function uncontended(label, fn) {
  for (let attempt = 0; ; attempt++) {
    let busy = await externalCompilers();
    const waitStart = Date.now();
    let heartbeat = 0;
    const observed = new Map();
    while (busy.length) {
      for (const p of busy) observed.set(p.pid, p);
      if (Date.now() - heartbeat > 15_000) {
        process.stderr.write(`  ${label}: waiting for external compiler(s): ${busy.map(p => `${basename(p.executable)}:${p.pid}`).join(', ')}\n`);
        heartbeat = Date.now();
      }
      await sleep(contention.pollMs);
      busy = await externalCompilers();
    }
    if (observed.size) contention.waits.push({ label, waitedMs: Date.now() - waitStart, processes: [...observed.values()] });
    let running = true;
    let pollError;
    const overlap = new Map();
    let timer;
    let wake;
    const monitor = (async () => {
      while (running) {
        await new Promise(resolve => { wake = resolve; timer = setTimeout(resolve, contention.pollMs); });
        if (!running) break;
        try { for (const p of await externalCompilers()) overlap.set(p.pid, p); }
        catch (err) { pollError = err; break; }
      }
    })();
    let result;
    try { result = await fn(attempt); }
    finally { running = false; clearTimeout(timer); wake?.(); await monitor; }
    if (pollError) throw pollError;
    for (const p of await externalCompilers()) overlap.set(p.pid, p);
    if (!overlap.size) return result;
    contention.discarded.push({ label, attempt, result, processes: [...overlap.values()] });
    process.stderr.write(`  ${label}: discarded overlapping sample; retrying after external compiler work\n`);
  }
}
