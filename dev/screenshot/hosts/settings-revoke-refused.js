// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Reichweite wurde nicht entfernt
//
// A runtime for the App-access page where the daemon REFUSES a revoke.
//
// The sentence under test used to be English prose set from the store and
// rendered verbatim, so a German reader who pressed Remove against a daemon that
// declined read "Could not remove that reach. Nothing changed." in the middle of
// a German page. It is a catalogue key now, and this is what proves it: the only
// way to that snackbar is a revoke the backend turns down, which no route walk
// reaches.
//
// `OK: required` is the realistic refusal rather than an invented one - it is
// what `revoke_at` answers for a reach the app declared essential.
//
// NB the command RESOLVES with that token rather than rejecting. `applyReaches`
// treats a thrown error as applied on purpose (so the affordance still works
// under vite with no daemon), so a fixture that rejects would photograph the
// success path while looking like a refusal.
(function () {
  var CEILING = JSON.stringify({
    read: [{ entity_type: "system.File", fields: null, exclude_fields: [] }],
    write: [],
    relations: [],
    instance: "All",
  });
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "access_grants") {
        return Promise.resolve([
          {
            id: "0192-0001",
            app_id: "dev.arlen.notes",
            declared_ceiling: CEILING,
            required: false,
            identity_verified: true,
            live: true,
            revoked: false,
            superseded: false,
            issued_at: 1780000000000000,
            expires_at: 0,
            reach: ["File"],
            source: "capability-token",
            consent_class: "",
            consent_scope: "",
          },
        ]);
      }
      // The page's two other reads. Both answer honestly rather than refusing:
      // a refusal here puts a SECOND notice on the page, and a picture with two
      // refusals in it is evidence about neither.
      if (cmd === "list_capsules") return Promise.resolve([]);
      if (cmd === "settings_sensing_state") {
        return Promise.resolve({ camera: [], microphone: [], location: [], screenCapture: null });
      }
      if (cmd === "revoke_reach") return Promise.resolve("OK: required");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: press the line's Remove, then the dialog's.
//
// THREE buttons on this page say a form of the same word and the first cut took
// the wrong one: `button.remove` is also the principal's "Alle entfernen", which
// comes first in reading order, so the fixture opened the remove-ALL dialog and
// photographed that instead. The line's button is the one whose whole text is
// `Entfernen`; the dialog's confirm is the one that carries no `remove` class.
(function () {
  var tries = 0;
  var stage = 0;
  function tick() {
    if (stage === 0) {
      var all = document.querySelectorAll("button.remove");
      for (var i = 0; i < all.length; i++) {
        if ((all[i].textContent || "").trim() === "Entfernen") {
          all[i].click();
          stage = 1;
          break;
        }
      }
    } else if (stage === 1) {
      var all = document.querySelectorAll("button");
      for (var i = 0; i < all.length; i++) {
        var b = all[i];
        if (b.classList.contains("remove")) continue;
        if ((b.textContent || "").trim() === "Entfernen") { b.click(); return; }
      }
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 150); });
})();
