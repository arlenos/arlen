// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Die Dateiauswahl ließ sich nicht öffnen
//
// A machine with no mailbox, where the window offers to open a message file
// and the file chooser cannot be shown: no portal on the session bus, which is
// what a bare compositor without the portal service answers.
//
// The offer itself is the other thing this photographs: the empty window's
// sentence and the button under it stand in the same picture as the refusal.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // A machine with no maildir, opened from the launcher: the state the offer
      // to open a message file is made in.
      if (cmd === "mail_folders") return Promise.resolve([]);
      if (cmd === "mail_store") return Promise.resolve("/home/you/Maildir");
      if (cmd === "launch_file") return Promise.resolve(null);
      // The one refusal this fixture is about, in the shape the plugin sends:
      // a tagged object, not a sentence.
      if (cmd === "plugin:arlen-portal|pick_file") {
        return Promise.reject({ type: "portalUnavailable", message: "org.freedesktop.portal.Desktop was not provided by any .service files" });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

(function () {
  var tries = 0;
  function tick() {
    var open = document.getElementById("mail-open");
    if (open) { open.click(); return; }
    if (++tries < 80) setTimeout(tick, 100);
  }
  tick();
})();
