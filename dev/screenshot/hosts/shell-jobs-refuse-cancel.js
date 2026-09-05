// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: ließ sich nicht abbrechen
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
  function named(names) {
    return Array.from(document.querySelectorAll("button")).find(function (b) {
      return names.indexOf(b.getAttribute("aria-label") || "") !== -1;
    });
  }
  function cancel() {
    var btn = named(["Abbrechen", "Cancel"]);
    if (btn) return btn.click();
    if (tries++ < 60) setTimeout(cancel, 100);
  }
  function tick() {
    // IT USED TO CLICK A BUTTON NOBODY COULD SEE. The jobs live in
    // `NotificationsPopover`, and going straight for Cancel FOUND one - because a
    // closed popover keeps its contents in the layout at `visibility: hidden`,
    // and `.click()` works on a button no person could press. Measured 6
    // September: the whole chain from the button up to `.pop-body` was hidden, so
    // the refusal this fixture exists to show was rendered invisible in a panel
    // that was never open, at every width and every settle. A fixture driving
    // hidden UI is a fixture describing a screen nobody sees.
    // `_jobstest` renders the zone on its own, with no popover around it, and
    // that is the route this fixture is meant to be pointed at:
    //
    //   probe-host.sh shell-jobs-refuse-cancel http://localhost:1420/_jobstest …
    //
    // On the shell's own route the trigger is clicked first, which is the right
    // intent and does not work yet - bits-ui's popover does not open for a
    // synthetic click, the same wall the file manager's context menu sits behind
    // - so the EXPECT check stays red there rather than passing on hidden UI.
    if (named(["Abbrechen", "Cancel"])) return cancel();
    var trigger = named(["Mitteilungen", "Notifications"]);
    if (trigger) {
      trigger.click();
      return setTimeout(cancel, 300);
    }
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 500); });
})();
