// Receives HMR messages and fetches the current page fragment when needed.
function refetchCurrentFragment() {
  var currentPath = location.pathname + location.search;
  fetch(currentPath, {
    headers: { Accept: "text/x-deka-fragment" },
    credentials: "same-origin",
  })
    .then(function (response) {
      if (!response.ok) {
        return null;
      }
      return response.json();
    })
    .then(function (fragment) {
      if (fragment && typeof fragment.html === "string") {
        patchElementHtml("#app", fragment.html);
        if (typeof fragment.title === "string" && fragment.title !== "") {
          document.title = fragment.title;
        }
        if (typeof fragment.head === "string" && fragment.head !== "") {
          document.head.insertAdjacentHTML("beforeend", fragment.head);
        }
        return;
      }
      location.reload();
    })
    .catch(function () {
      location.reload();
    });
}

function subscribeToCurrentPath() {
  try {
    hmrSocket.send(
      JSON.stringify({
        type: "subscribe",
        path: location.pathname + location.search,
      })
    );
  } catch (_) {}
}

function applyPatchMessage(message) {
  if (!message || !Array.isArray(message.ops) || message.ops.length === 0) {
    refetchCurrentFragment();
    return;
  }
  for (var index = 0; index < message.ops.length; index++) {
    var operation = message.ops[index] || {};
    if (operation.op === "set_html") {
      if (operation.island) {
        patchIslandHtml(
          operation.island,
          operation.occurrence || 1,
          operation.html || ""
        );
      } else {
        patchElementHtml(operation.selector || "#app", operation.html || "");
      }
      continue;
    }
    refetchCurrentFragment();
    return;
  }
}

hmrSocket.onopen = function () {
  subscribeToCurrentPath();
};
hmrSocket.onmessage = function (event) {
  try {
    var message = JSON.parse(event.data || "{}");
    if (message.type === "patch") {
      applyPatchMessage(message);
      return;
    }
    if (message.type === "reload") {
      refetchCurrentFragment();
      return;
    }
  } catch (_) {
    refetchCurrentFragment();
  }
};
window.addEventListener("popstate", function () {
  subscribeToCurrentPath();
});
hmrSocket.onclose = function () {};
  } catch (_) {}
})();
