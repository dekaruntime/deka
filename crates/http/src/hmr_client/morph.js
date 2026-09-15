// Dev-only DOM morph: patch live markup against new server HTML while
// leaving hydrated <deka-island> subtrees untouched. Island source edits
// full-reload instead (see socket.js, reason "island-source").
function isHmrClientNode(node) {
  if (!node || node.nodeType !== 1) {
    return false;
  }
  var id = node.id || "";
  return (
    id === "__deka_hmr_client" ||
    id === "__deka_refresh_preamble" ||
    id === "__deka_react_importmap"
  );
}

function isProtectedNode(node) {
  if (!node || node.nodeType !== 1) {
    return false;
  }
  if (isHmrClientNode(node)) {
    return true;
  }
  return node.tagName === "SCRIPT" || node.tagName === "LINK" || node.tagName === "STYLE";
}

function isIslandElement(node) {
  if (!node || node.nodeType !== 1) {
    return false;
  }
  return (
    node.tagName === "DEKA-ISLAND" ||
    node.hasAttribute("data-deka-island") ||
    node.hasAttribute("data-deka-island-id")
  );
}

function islandKey(node) {
  return (
    (node.getAttribute &&
      (node.getAttribute("data-deka-island") ||
        node.getAttribute("data-deka-island-id"))) ||
    ""
  );
}

function isIslandStartComment(node) {
  return (
    node &&
    node.nodeType === 8 &&
    String(node.data || "").indexOf(DEKA_ISLAND_START_PREFIX) === 0
  );
}

function morphAttrs(live, incoming) {
  if (!live.attributes || !incoming.attributes) {
    return;
  }
  var keep = {};
  for (var i = 0; i < incoming.attributes.length; i++) {
    var attr = incoming.attributes[i];
    keep[attr.name] = true;
    if (live.getAttribute(attr.name) !== attr.value) {
      live.setAttribute(attr.name, attr.value);
    }
  }
  var remove = [];
  for (var j = 0; j < live.attributes.length; j++) {
    var name = live.attributes[j].name;
    if (!keep[name]) {
      remove.push(name);
    }
  }
  for (var k = 0; k < remove.length; k++) {
    live.removeAttribute(remove[k]);
  }
}

function sameIdentity(live, incoming) {
  if (!live || !incoming || live.nodeType !== incoming.nodeType) {
    return false;
  }
  if (live.nodeType === 3) {
    return true;
  }
  if (live.nodeType === 8) {
    if (isIslandStartComment(live) || isIslandStartComment(incoming)) {
      // Compare marker signatures, not raw data: a volatile cache token
      // must not read as a different island (that would insert a clone of
      // an already-hydrated island and delete the live one).
      return islandMarkerSignature(live.data) === islandMarkerSignature(incoming.data);
    }
    return true;
  }
  if (live.nodeType !== 1) {
    return false;
  }
  if (isProtectedNode(live) && isProtectedNode(incoming)) {
    return (live.id || "") === (incoming.id || "") && live.tagName === incoming.tagName;
  }
  if (live.nodeName !== incoming.nodeName) {
    return false;
  }
  if (isIslandElement(live) || isIslandElement(incoming)) {
    return (
      isIslandElement(live) &&
      isIslandElement(incoming) &&
      islandKey(live) !== "" &&
      islandKey(live) === islandKey(incoming)
    );
  }
  if (live.id || incoming.id) {
    return live.id === incoming.id;
  }
  var liveDeka =
    (live.getAttribute && live.getAttribute("data-deka-id")) || "";
  var incomingDeka =
    (incoming.getAttribute && incoming.getAttribute("data-deka-id")) || "";
  if (liveDeka || incomingDeka) {
    return liveDeka !== "" && liveDeka === incomingDeka;
  }
  var liveName = (live.getAttribute && live.getAttribute("name")) || "";
  var incomingName =
    (incoming.getAttribute && incoming.getAttribute("name")) || "";
  if (liveName && liveName === incomingName) {
    return true;
  }
  return true;
}

function skipIslandCommentRange(node) {
  if (!isIslandStartComment(node)) {
    return node;
  }
  var depth = 1;
  var cursor = node.nextSibling;
  while (cursor) {
    if (cursor.nodeType === 8) {
      var data = String(cursor.data || "");
      if (data.indexOf(DEKA_ISLAND_START_PREFIX) === 0) {
        depth++;
      } else if (data.indexOf(DEKA_ISLAND_END_PREFIX) === 0) {
        depth--;
        if (!depth) {
          return cursor;
        }
      }
    }
    cursor = cursor.nextSibling;
  }
  return node;
}

// Stable identity of an island start marker, ignoring volatile fields.
// Grammar: "deka-island start:<b64 name> directive:<b64> [props:<b64>]
// [id:<b64>] [cache:<b64>] ..." — cache tokens may change per render and
// must not read as an island change, so they stay out of the signature.
function islandMarkerSignature(data) {
  return String(data || "")
    .split(" ")
    .filter(function (field) {
      return field.indexOf("cache:") !== 0;
    })
    .join(" ");
}

// Island signature sequence in document order (comment markers and
// <deka-island> elements). The morph can only preserve hydrated islands
// when the incoming markup references the exact same islands with the exact
// same props; a server-component edit that changes an island's props,
// directive, set, or order cannot be morphed in (hydrateRoot owns that
// subtree), so the caller full-reloads instead of keeping a stale island.
function collectIslandSignatures(root) {
  var signatures = [];
  (function walk(node) {
    var child = node.firstChild;
    while (child) {
      var next = child.nextSibling;
      if (child.nodeType === 8 && isIslandStartComment(child)) {
        signatures.push("c:" + islandMarkerSignature(child.data));
        next = skipIslandCommentRange(child).nextSibling;
      } else if (isIslandElement(child)) {
        signatures.push(
          "e:" +
            child.tagName +
            "|" +
            islandKey(child) +
            "|" +
            (child.getAttribute("data-deka-directive") || "") +
            "|" +
            (child.getAttribute("data-deka-props") || "")
        );
      } else if (child.nodeType === 1) {
        walk(child);
      }
      child = next;
    }
  })(root);
  return signatures;
}

function islandSignaturesEqual(live, incoming) {
  if (!Array.isArray(live) || !Array.isArray(incoming)) {
    return false;
  }
  if (live.length !== incoming.length) {
    return false;
  }
  for (var i = 0; i < live.length; i++) {
    if (live[i] !== incoming[i]) {
      return false;
    }
  }
  return true;
}

function morphNode(live, incoming) {
  if (!live || !incoming) {
    return;
  }
  if (isIslandElement(live)) {
    return;
  }
  if (isIslandStartComment(live)) {
    return;
  }
  if (live.nodeType === 3 || live.nodeType === 8) {
    if (live.nodeValue !== incoming.nodeValue) {
      live.nodeValue = incoming.nodeValue;
    }
    return;
  }
  if (live.nodeType !== 1 || incoming.nodeType !== 1) {
    return;
  }
  if (isProtectedNode(live)) {
    return;
  }
  morphAttrs(live, incoming);
  morphChildren(live, incoming);
}

function morphChildren(liveParent, incomingParent) {
  var incomingNode = incomingParent.firstChild;
  var liveNode = liveParent.firstChild;
  while (incomingNode) {
    var nextIncoming = incomingNode.nextSibling;
    if (isProtectedNode(incomingNode)) {
      incomingNode = nextIncoming;
      continue;
    }
    while (liveNode && isProtectedNode(liveNode)) {
      liveNode = liveNode.nextSibling;
    }
    if (
      liveNode &&
      isIslandElement(liveNode) &&
      isIslandElement(incomingNode) &&
      islandKey(liveNode) === islandKey(incomingNode)
    ) {
      liveNode = liveNode.nextSibling;
      incomingNode = nextIncoming;
      continue;
    }
    if (liveNode && isIslandStartComment(liveNode) && isIslandStartComment(incomingNode)) {
      if (
        islandMarkerSignature(liveNode.data) === islandMarkerSignature(incomingNode.data)
      ) {
        // Same island, possibly refreshed cache token: sync the marker so
        // the skipped range stays paired with the incoming markup, then
        // leave the hydrated body untouched.
        if (liveNode.data !== incomingNode.data) {
          liveNode.data = incomingNode.data;
        }
        liveNode = skipIslandCommentRange(liveNode).nextSibling;
        incomingNode = nextIncoming;
        continue;
      }
    }
    if (!liveNode) {
      liveParent.appendChild(incomingNode.cloneNode(true));
      incomingNode = nextIncoming;
      continue;
    }
    if (sameIdentity(liveNode, incomingNode)) {
      if (!isIslandElement(liveNode) && !isIslandStartComment(liveNode)) {
        morphNode(liveNode, incomingNode);
      }
      if (isIslandStartComment(liveNode)) {
        liveNode = skipIslandCommentRange(liveNode).nextSibling;
      } else {
        liveNode = liveNode.nextSibling;
      }
      incomingNode = nextIncoming;
      continue;
    }
    var found = null;
    for (var look = liveNode.nextSibling; look; look = look.nextSibling) {
      if (isProtectedNode(look)) {
        continue;
      }
      if (sameIdentity(look, incomingNode)) {
        found = look;
        break;
      }
    }
    if (found) {
      liveParent.insertBefore(found, liveNode);
      if (!isIslandElement(found) && !isIslandStartComment(found)) {
        morphNode(found, incomingNode);
      }
      liveNode = isIslandStartComment(found)
        ? skipIslandCommentRange(found).nextSibling
        : found.nextSibling;
    } else {
      liveParent.insertBefore(incomingNode.cloneNode(true), liveNode);
    }
    incomingNode = nextIncoming;
  }
  while (liveNode) {
    var nextLive = liveNode.nextSibling;
    if (!isProtectedNode(liveNode)) {
      if (isIslandStartComment(liveNode)) {
        var end = skipIslandCommentRange(liveNode);
        nextLive = end.nextSibling;
        var range = document.createRange();
        range.setStartBefore(liveNode);
        range.setEndAfter(end);
        range.deleteContents();
      } else {
        liveParent.removeChild(liveNode);
      }
    }
    liveNode = nextLive;
  }
}

function applyHtmlUpdate(message) {
  var html = String((message && message.html) || "");
  var selector = (message && message.selector) || "#app";
  var live = queryElement(selector);
  if (!live) {
    live = document.body;
  }
  if (!live) {
    location.reload();
    return;
  }

  var scrollY = window.scrollY || window.pageYOffset || 0;
  var focusedFieldState = captureFocusedFieldState();
  var fieldStates = captureFormFieldValues();
  var template = document.createElement("template");
  template.innerHTML = normalizeShadowRootMode(html);
  // An html-update can only preserve hydrated islands when the edit left
  // the island boundary untouched. A server-component edit that changed an
  // island's props/directive or added/removed/reordered one cannot be
  // reflected by morphing (hydrateRoot owns that subtree), so take the same
  // honest full-reload fallback used for island-source edits — a silent
  // stale island is worse than a reload.
  if (
    !islandSignaturesEqual(
      collectIslandSignatures(live),
      collectIslandSignatures(template.content)
    )
  ) {
    location.reload();
    return;
  }
  morphChildren(live, template.content);
  window.scrollTo(0, scrollY);
  restoreFormFieldValues(fieldStates);
  restoreFocusedFieldState(focusedFieldState);
}
