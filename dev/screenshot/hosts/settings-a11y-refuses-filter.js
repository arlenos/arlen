// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Änderung wurde nicht gespeichert
//
// A runtime for the accessibility page where the filter READS and the write is
// refused.
//
// The screen-filter store carried the config store's defect in full: the catch
// put the reason in `error` and then called `loadFilter()` to roll the switch
// back, and `loadFilter()` clears `error` on its first line - so the failure was
// erased by the recovery. And this page renders `error` through
// `ConfigUnavailable`, whose sentence is that the settings could not be READ and
// the values below are defaults, which after a successful rollback is false
// twice over.
//
// So what a person met on the most safety-relevant page in this app was an
// invert-colours switch sliding back on its own, silently.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // Both reads answer, so the page shows a real state rather than defaults.
      if (cmd === "accessibility_filter_get") {
        return Promise.resolve({ inverted: false, colorFilter: null });
      }
      if (cmd === "config_get" || cmd === "config_get_default") {
        return Promise.resolve({ zoom: {}, xkb_config: {}, layout: {} });
      }
      // The one refusal this fixture is about.
      if (cmd === "accessibility_filter_set") {
        return Promise.reject("the compositor did not take it (os error 111)");
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: flip Invert colours.
(function () {
  var tries = 0;
  function tick() {
    var row = document.getElementById("invert-colors");
    var sw = row ? row.querySelector('button[role="switch"], button[aria-label]') : null;
    if (sw) { sw.click(); return; }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
