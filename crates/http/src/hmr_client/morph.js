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
    String(node.data || "").indexOf("deka-island start:") === 0
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
      return String(live.data || "") === String(incoming.data || "");
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
      if (data.indexOf("deka-island start:") === 0) {
        depth++;
      } else if (data.indexOf("deka-island end:") === 0) {
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
      if (String(liveNode.data || "") === String(incomingNode.data || "")) {
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
  morphChildren(live, template.content);
  window.scrollTo(0, scrollY);
  restoreFormFieldValues(fieldStates);
  restoreFocusedFieldState(focusedFieldState);
}
