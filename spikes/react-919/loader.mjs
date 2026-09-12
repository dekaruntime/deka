// Spike-only resolver. Neither a product ui package nor a runtime change.
import fs from 'node:fs';
const root = new URL('./', import.meta.url);
const lock = JSON.parse(fs.readFileSync(new URL('deka.lock',root)));
export function resolve(specifier, context, nextResolve) {
  if((specifier === 'ui/jsx' || specifier === 'ui/reactive')) return {url:new URL('jsx-adapter.mjs',root).href,shortCircuit:true};
  if(specifier.startsWith('@js/')) {
    if(!Object.hasOwn(lock.packages,specifier)) throw Error(`Unlocked import ${specifier}`);
    const name = specifier.slice(4);
    if(!/^[a-zA-Z0-9_-]+$/.test(name)) throw Error(`Unsupported import ${specifier}`);
    return {url:new URL(`js_modules/${name}/index.mjs`,root).href,shortCircuit:true};
  }
  return nextResolve(specifier,context);
}
