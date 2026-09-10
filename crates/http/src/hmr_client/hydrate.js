// Imports the client hydrator for HMR. The Rust injector owns the surrounding
// module script tag, so every hmr_client fragment is JavaScript-only.
var hmrHydrate = null;
import("ui/client")
  .then(function (hydrateModule) {
    if (hydrateModule && typeof hydrateModule.hydrate === "function") {
      hmrHydrate = hydrateModule.hydrate;
    }
  })
  .catch(function () {});
