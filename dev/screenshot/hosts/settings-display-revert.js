// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Display-Änderungen behalten?
//
// A runtime for the display page with the 15-second revert modal OPEN.
//
// It is the one Settings dialog no route or click could reach: the modal opens
// only after a display change has actually been applied, so under vite the
// apply refuses and the modal never exists. It was therefore the only one of the
// three dialogs whose focus ring - WebKit's default, drawn round the card once
// the card takes focus - could not be photographed.
//
// The saved-layout Apply is the way in rather than the page's own Apply button,
// which is gated on `dirty` and would need a setting changed first. Same modal,
// one gesture.
(function () {
  var SNAPSHOT = [
    { connector: "DP-1", modeIndex: 0, position: { x: 0, y: 0 }, scale: 1, transform: "normal", enabled: true },
  ];
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "display_get_monitors") {
        return Promise.resolve([
          {
            connector: "DP-1",
            make: "Example",
            model: "E24",
            serial: "S1",
            physicalSizeMm: [530, 300],
            modes: [{ width: 1920, height: 1080, refreshMhz: 60000, preferred: true }],
            currentMode: 0,
            preferredMode: 0,
            position: { x: 0, y: 0 },
            scale: 1,
            transform: "normal",
            enabled: true,
            mirroring: null,
            vrr: "disabled",
            primary: true,
            maxBpc: 8,
          },
        ]);
      }
      if (cmd === "display_profiles_list") {
        return Promise.resolve([
          {
            id: "p1",
            label: "Schreibtisch",
            outputSet: [{ connector: "DP-1", make: "Example", model: "E24" }],
            lastUsed: null,
            // NOT current, or the Apply button is disabled and the fixture
            // photographs a page with nothing pressed.
            isCurrent: false,
          },
        ]);
      }
      if (cmd === "display_profile_apply") {
        return Promise.resolve({ requestId: "r1", snapshot: SNAPSHOT });
      }
      // The page's two other sections. Answered rather than refused: a refusal
      // here puts a second failure on screen and a picture with two of them is
      // evidence about neither.
      if (cmd === "brightness_get_devices") return Promise.resolve([]);
      if (cmd === "night_light_get_state") {
        // EVERY field of `NightLightState`, not the four this fixture cares
        // about - `check-fixture-answers-whole` caught the short version, which
        // is the same shape that once made the mail reader throw mid-render and
        // look like a live-mailbox defect.
        return Promise.resolve({
          enabled: false,
          temperature: 4000,
          schedule: "off",
          custom_start: 1200,
          custom_end: 420,
          latitude: 0,
          longitude: 0,
        });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: press the apply button in the display page's action row. The modal
// counts down from 15, so the shot has to be taken well inside that; the driver
// fires as soon as the button exists rather than on a timer.
//
// BY POSITION, not by the word. This pressed a button reading `Anwenden`, and
// that is a sentence the catalogue owns - five sibling fixtures broke on
// 10 September because the word they pressed had been improved to something
// else, silently. The saved-layout row's apply is the FIRST button in
// `.row-actions` (`displays/ProfileSection.svelte`: apply, then rename, then the
// rest), and the enabled check stays because that button is disabled on the
// layout that is already current.
//
// The first attempt at this used `.action-row button`, which is the display
// page's own apply for a dirty draft - a different button on the same page, and
// the fixture stopped reaching its state. Written down because the two are easy
// to confuse from the markup alone.
(function () {
  var tries = 0;
  function tick() {
    var all = document.querySelectorAll(".row-actions button");
    for (var i = 0; i < all.length; i++) {
      if (!all[i].disabled) {
        all[i].click();
        return;
      }
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 150); });
})();
