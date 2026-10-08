// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: 3
//
// The workspace strip answering a refused move (`Super+Shift+N` with nothing to
// move): the compositor emits `arlen://workspace-move-refused` and the pill
// highlight goes to the target and comes back. There is no text on purpose, so
// the expected text only proves the strip came up with all three workspaces; the
// motion itself is for a probe to read, by calling `window.__fireRefused()` and
// sampling the highlight before, during and after.
//
// Speaks the event plugin the same way `shell-workspace-overlay.js` does.
(function () {
  var listeners = {};

  function emit(name, payload) {
    (listeners[name] || []).forEach(function (h) {
      h({ event: name, id: 0, payload: payload });
    });
  }

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      if (cmd === "plugin:event|listen") {
        var key = args && args.event;
        (listeners[key] = listeners[key] || []).push(args.handler);
        return Promise.resolve(1);
      }
      if (cmd === "plugin:event|unlisten") return Promise.resolve();
      if (cmd === "get_workspaces") {
        return Promise.resolve([
          { id: "w1", group_id: "g1", name: "1", active: true, output_connectors: ["DP-1"] },
          { id: "w2", group_id: "g1", name: "2", active: false, output_connectors: ["DP-1"] },
          { id: "w3", group_id: "g1", name: "3", active: false, output_connectors: ["DP-1"] },
        ]);
      }
      if (cmd === "get_windows") return Promise.resolve([]);
      if (cmd === "set_popover_input_region") return Promise.resolve(true);
      if (cmd === "get_focus_state") return Promise.resolve(null);
      if (cmd === "qs_layout_get") return Promise.resolve({ tile: [] });
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // For a probe: whether the strip has subscribed yet, so a probe can wait for it
  // rather than fire into nothing - and then refuse a move to the third
  // workspace, on DP-1.
  window.__refusedReady = function () {
    return !!listeners["arlen://workspace-move-refused"];
  };
  window.__fireRefused = function () {
    if (!listeners["arlen://workspace-move-refused"]) return false;
    emit("arlen://workspace-move-refused", { output: "DP-1", target: 2 });
    return true;
  };
})();
