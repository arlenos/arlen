// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Änderung wurde nicht gespeichert
//
// A runtime for a settings page whose READ works and whose WRITE is refused.
//
// This is the state every control in this app can reach and none of them could
// show. `setValue` put the reason in the store's `error` and then called `load()`
// to roll the value back - and `load()` clears `error` on its first line, so the
// failure was erased by the recovery. What a person met was a switch they had
// just moved sliding back on its own, with nothing said, anywhere.
//
// Neither of the other two host states reaches it: with no runtime the store
// short-circuits on `!tauriAvailable` and never writes at all, and with
// `--stub-host` the READ refuses too, so the page says its values are defaults
// and the write is beside the point.
(function () {
  var COMPOSITOR = {
    layout: { mode: "floating", inner_gap: 8, outer_gap: 8, smart_gaps: false, tiled_headers: false, window_rules: [] },
    xkb_config: { layout: "de" },
    workspaces: {},
    system_actions: {},
    keybindings: {},
  };
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // Both reads answer, so the page is a real one rather than a page of
      // defaults - which is the OTHER sentence and a different fixture.
      if (cmd === "config_get" || cmd === "config_get_default") {
        return Promise.resolve(COMPOSITOR);
      }
      // The one refusal this fixture is about. A read-only mount, a directory
      // that is not writable, a daemon that will not take it - the surface owes
      // the same sentence for all three.
      if (cmd === "config_set") return Promise.reject("Read-only file system (os error 30)");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: flip the smart-gaps switch. Polls for the control rather than firing
// on a timer, so a slow first paint moves the shot instead of producing an empty
// one that reads as a finding.
(function () {
  var tries = 0;
  function tick() {
    var row = document.getElementById("smart-gaps");
    var sw = row ? row.querySelector('button[role="switch"], button[aria-label]') : null;
    if (sw) { sw.click(); return; }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
