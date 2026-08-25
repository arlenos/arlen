// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// A runtime where the Activity zone LISTS a job and refuses to cancel it. The
// zone's own `if (tauriAvailable)` guard means a refusal cannot be reached with
// no runtime, and --stub-host loses the list, so this is the only way to see the
// line the zone shows when a cancel does not take.
(function () {
  var JOBS = [
    {
      id: "fm-copy",
      title: "240 Fotos werden auf den Stick kopiert",
      appId: "org.arlen.files",
      appLabel: "Dateien",
      fraction: 0.35,
      state: "running",
      metrics: [{ processed: 84, total: 240, unit: "Dateien" }],
      killable: true,
      suspendable: true,
    },
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "list_jobs") return Promise.resolve(JOBS);
      if (cmd === "cancel_job" || cmd === "pause_job" || cmd === "resume_job") {
        return Promise.reject("the job daemon is not accepting that (os error 111)");
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Press Cancel on the one job. The button is icon-only, so it is found by its
  // accessible name - which is also a check that the name is there.
  var tries = 0;
  function tick() {
    var btn = Array.from(document.querySelectorAll("button")).find(function (b) {
      var n = b.getAttribute("aria-label") || "";
      return n === "Abbrechen" || n === "Cancel";
    });
    if (btn) return btn.click();
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 500); });
})();
