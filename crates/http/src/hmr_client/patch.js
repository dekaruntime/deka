// Applies HMR fragment operations while preserving scroll, focus, and form state.
function normalizeShadowRootMode(html) {
  return String(html || "").replace(
    /shadowrootmode=/gi,
    "data-shadowrootmode="
  );
}

function patchElementHtml(selector, html) {
  var targetNode = queryElement(selector || "#app");
  if (!targetNode) {
    location.reload();
    return;
  }

  var scrollY = window.scrollY || window.pageYOffset || 0;
  var focusedFieldState = captureFocusedFieldState();
  var fieldStates = captureFormFieldValues();
  var normalizedHtml = normalizeShadowRootMode(html);
  if (
    targetNode.matches &&
    targetNode.matches("[data-deka-island-id],deka-island") &&
    targetNode.shadowRoot
  ) {
    targetNode.shadowRoot.innerHTML = normalizedHtml;
  } else {
    targetNode.innerHTML = normalizedHtml;
  }
  if (typeof hmrHydrate === "function") {
    hmrHydrate(targetNode);
  }
  window.scrollTo(0, scrollY);
  restoreFormFieldValues(fieldStates);
  restoreFocusedFieldState(focusedFieldState);
}

function patchIslandHtml(islandName, occurrence, html) {
  var encodedIslandName = "";
  try {
    encodedIslandName = btoa(
      unescape(encodeURIComponent(String(islandName || "")))
    );
  } catch (_) {}
  if (!encodedIslandName) {
    location.reload();
    return;
  }
  if (typeof document.createTreeWalker !== "function") {
    location.reload();
    return;
  }

  var commentWalker = document.createTreeWalker(document.body, 128);
  var startComment = null;
  var comment;
  var occurrenceCount = 0;
  while ((comment = commentWalker.nextNode())) {
    var commentData = String(comment.data || "");
    if (commentData.indexOf("deka-island start:") !== 0) {
      continue;
    }
    var encodedCommentName = commentData.substring(
      18,
      commentData.indexOf(" ", 18)
    );
    if (encodedCommentName !== encodedIslandName) {
      continue;
    }
    occurrenceCount++;
    if (occurrenceCount === (occurrence || 1)) {
      startComment = comment;
      break;
    }
  }
  if (!startComment) {
    location.reload();
    return;
  }

  var depth = 1;
  var endComment = null;
  var sibling = startComment.nextSibling;
  while (sibling) {
    if (sibling.nodeType === 8) {
      var siblingData = String(sibling.data || "");
      if (siblingData.indexOf("deka-island start:") === 0) {
        depth++;
      } else if (siblingData.indexOf("deka-island end:") === 0) {
        depth--;
        if (!depth) {
          endComment = sibling;
          break;
        }
      }
    }
    sibling = sibling.nextSibling;
  }
  if (!endComment) {
    location.reload();
    return;
  }

  var scrollY = window.scrollY || window.pageYOffset || 0;
  var focusedFieldState = captureFocusedFieldState();
  var fieldStates = captureFormFieldValues();
  var template = document.createElement("template");
  template.innerHTML = normalizeShadowRootMode(html);
  var parentNode = startComment.parentNode;
  var currentNode = startComment.nextSibling;
  while (currentNode && currentNode !== endComment) {
    var nextNode = currentNode.nextSibling;
    parentNode.removeChild(currentNode);
    currentNode = nextNode;
  }
  parentNode.insertBefore(template.content, endComment);
  if (typeof hmrHydrate === "function") {
    hmrHydrate(parentNode);
  }
  window.scrollTo(0, scrollY);
  restoreFormFieldValues(fieldStates);
  restoreFocusedFieldState(focusedFieldState);
}
