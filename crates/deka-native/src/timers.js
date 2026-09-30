// Install before the shared bootstrap hides Deno. Closures retain only timer
// functions, not a publicly reachable raw host-op table.
(() => {
  const {createTimer, cancelTimer} = Deno.core;
  globalThis.setTimeout = (callback, delay = 0, ...args) => {
    if (typeof callback !== 'function') throw new TypeError('setTimeout requires a function');
    return createTimer(callback, delay, args, false, true);
  };
  globalThis.clearTimeout = id => cancelTimer(id);
})();
