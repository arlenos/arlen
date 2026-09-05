// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Suche ist nicht gelaufen
//
// A runtime where the terminal comes up and the history search is refused - the
// state a person meets when the history store is unreachable. --stub-host alone
// would do it, but the palette has to be OPENED (Ctrl+R) for the line to render,
// so the host performs the gesture.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Ctrl+R opens the palette; the layout binds it on the window.
  window.addEventListener("load", function () {
    setTimeout(function () {
      window.dispatchEvent(new KeyboardEvent("keydown", {
        key: "r", code: "KeyR", keyCode: 82, which: 82,
        ctrlKey: true, bubbles: true, cancelable: true,
      }));
    }, 800);
  });
})();
