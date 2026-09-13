// Must run before any React import so Fast Refresh can hook the dispatcher.
import * as RefreshRuntime from "/_deka/react/refresh-runtime.js";
RefreshRuntime.injectIntoGlobalHook(window);
window.$RefreshReg$ = function () {};
window.$RefreshSig$ = function () {
  return function (type) {
    return type;
  };
};
window.__deka_refresh_runtime = RefreshRuntime;
