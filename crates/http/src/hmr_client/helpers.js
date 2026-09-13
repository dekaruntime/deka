// Opens the development HMR socket and preserves focus and form state across patches.
(function () {
  try {
    var socketProtocol = location.protocol === "https:" ? "wss" : "ws";
    var hmrSocket = new WebSocket(
      socketProtocol + "://" + location.host + "/_deka/hmr"
    );

    function queryElement(selector) {
      return document.querySelector(selector || "#app") || document.body;
    }

    function normalizeShadowRootMode(html) {
      return String(html || "").replace(
        /shadowrootmode=/gi,
        "data-shadowrootmode="
      );
    }

    function isOutsideIsland(node) {
      if (!node || !node.closest) {
        return true;
      }
      return !node.closest("deka-island, [data-deka-island], [data-deka-island-id]");
    }

    function escapeAttributeSelectorValue(value) {
      return String(value || "")
        .replace(/\\/g, "\\\\")
        .replace(/"/g, '\\"');
    }

    function captureFocusedFieldState() {
      var activeElement = document.activeElement;
      if (!activeElement || !activeElement.closest) {
        return null;
      }
      if (!isOutsideIsland(activeElement)) {
        return null;
      }
      if (!activeElement.closest("#app") && !document.body.contains(activeElement)) {
        return null;
      }

      return {
        id: activeElement.id || "",
        name: activeElement.getAttribute("name") || "",
        deka: activeElement.getAttribute("data-deka-id") || "",
        start:
          typeof activeElement.selectionStart === "number"
            ? activeElement.selectionStart
            : null,
        end:
          typeof activeElement.selectionEnd === "number"
            ? activeElement.selectionEnd
            : null,
      };
    }

    function restoreFocusedFieldState(focusedFieldState) {
      if (!focusedFieldState) {
        return;
      }

      var field = null;
      if (focusedFieldState.id) {
        field = document.getElementById(focusedFieldState.id);
      }
      if (!field && focusedFieldState.deka) {
        field = document.querySelector(
          '#app [data-deka-id="' +
            escapeAttributeSelectorValue(focusedFieldState.deka) +
            '"]'
        );
      }
      if (!field && focusedFieldState.name) {
        field = document.querySelector(
          '#app [name="' +
            escapeAttributeSelectorValue(focusedFieldState.name) +
            '"]'
        );
      }
      if (!field || typeof field.focus !== "function") {
        return;
      }

      field.focus();
      if (
        focusedFieldState.start !== null &&
        focusedFieldState.end !== null &&
        typeof field.setSelectionRange === "function"
      ) {
        try {
          field.setSelectionRange(
            focusedFieldState.start,
            focusedFieldState.end
          );
        } catch (_) {}
      }
    }

    function captureFormFieldValues() {
      var appRoot = queryElement("#app") || document.body;
      if (!appRoot) {
        return [];
      }

      var fieldStates = [];
      var fields = appRoot.querySelectorAll("input,textarea,select");
      for (var index = 0; index < fields.length; index++) {
        var field = fields[index];
        if (!isOutsideIsland(field)) {
          continue;
        }
        var id = field.id || "";
        var name = field.getAttribute("name") || "";
        var deka = field.getAttribute("data-deka-id") || "";
        if (!id && !name && !deka) {
          continue;
        }

        var type = (field.getAttribute("type") || "").toLowerCase();
        var fieldState = { id: id, name: name, deka: deka, type: type };
        if (type === "checkbox" || type === "radio") {
          fieldState.checked = !!field.checked;
        } else if (field.tagName === "SELECT") {
          fieldState.value = field.value;
        } else {
          fieldState.value = field.value;
        }
        fieldStates.push(fieldState);
      }
      return fieldStates;
    }

    function restoreFormFieldValues(fieldStates) {
      if (!Array.isArray(fieldStates) || fieldStates.length === 0) {
        return;
      }

      for (var index = 0; index < fieldStates.length; index++) {
        var fieldState = fieldStates[index] || {};
        var field = null;
        if (fieldState.id) {
          field = document.getElementById(fieldState.id);
        }
        if (!field && fieldState.deka) {
          field = document.querySelector(
            '#app [data-deka-id="' +
              escapeAttributeSelectorValue(fieldState.deka) +
              '"]'
          );
        }
        if (!field && fieldState.name) {
          field = document.querySelector(
            '#app [name="' +
              escapeAttributeSelectorValue(fieldState.name) +
              '"]'
          );
        }
        if (!field) {
          continue;
        }
        if (
          (fieldState.type === "checkbox" || fieldState.type === "radio") &&
          typeof fieldState.checked === "boolean"
        ) {
          field.checked = fieldState.checked;
          continue;
        }
        if (typeof fieldState.value !== "undefined") {
          field.value = fieldState.value;
        }
      }
    }
