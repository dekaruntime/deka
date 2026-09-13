function applyJsUpdate(message) {
  var runtime = window.__deka_refresh_runtime;
  if (!runtime || typeof runtime.performReactRefresh !== "function") {
    location.reload();
    return;
  }
  var modules = message && Array.isArray(message.modules) ? message.modules : [];
  if (modules.length === 0) {
    location.reload();
    return;
  }
  var chain = Promise.resolve();
  for (var i = 0; i < modules.length; i++) {
    (function (mod) {
      chain = chain.then(function () {
        var url = String(mod.url || "");
        if (!url) {
          throw new Error("missing module url");
        }
        var bust = url.indexOf("?") === -1 ? "?t=" + Date.now() : "&t=" + Date.now();
        return import(url + bust).then(function (ns) {
          if (!ns || ns.__dekaRefreshBoundary === false) {
            throw new Error("module is not a refresh boundary");
          }
          var names = Object.keys(ns).filter(function (key) {
            return (
              key !== "__dekaRefreshBoundary" &&
              key !== "__esModule" &&
              key !== "default"
            );
          });
          var values = names.map(function (key) {
            return ns[key];
          });
          if (typeof ns.default !== "undefined") {
            values.push(ns.default);
          }
          var hasComponent = false;
          for (var v = 0; v < values.length; v++) {
            if (runtime.isLikelyComponentType(values[v])) {
              hasComponent = true;
            } else if (
              values[v] !== null &&
              typeof values[v] === "object" &&
              values[v].$$typeof
            ) {
              continue;
            } else if (typeof values[v] === "function") {
              throw new Error("non-component export");
            }
          }
          if (!hasComponent) {
            throw new Error("no component export");
          }
        });
      });
    })(modules[i]);
  }
  chain
    .then(function () {
      runtime.performReactRefresh();
    })
    .catch(function () {
      location.reload();
    });
}
