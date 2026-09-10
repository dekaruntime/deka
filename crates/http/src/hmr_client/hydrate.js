<script id="__deka_hmr_client" type="module">
  // Imports the client hydrator for HMR; this file is an injected HTML script fragment.
  var hmrHydrate = null;
  import("ui/client")
    .then(function (hydrateModule) {
      if (hydrateModule && typeof hydrateModule.hydrate === "function") {
        hmrHydrate = hydrateModule.hydrate;
      }
    })
    .catch(function () {});
