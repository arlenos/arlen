// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Anordnung wurde nicht gespeichert
//
// A runtime for the topbar arrangement page where the inventory READS and the
// save is refused.
//
// The distinction is the whole point of this picture. This page deliberately
// does NOT roll back - a refused save keeps the order you dragged - so the
// screen and the actual bar disagree, and until today the only sentence it could
// show said the arrangement could not be READ and that "changes are paused".
// Neither was true: the items are right there, and nothing was paused.
(function () {
  // `kind` is on the wire and this panel does not read it - it encodes the same
  // distinction in `icon`. Answered anyway, because a real host always sends it
  // and a fixture that leaves out what it does not personally need is how the
  // mail one broke a reading surface this afternoon.
  var ITEMS = [
    { id: "clock", name: "Uhr", icon: "clock", kind: "applet", shown: true },
    { id: "network", name: "Netzwerk", icon: "network", kind: "applet", shown: true },
    { id: "battery", name: "Akku", icon: "battery", kind: "applet", shown: false },
  ];
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // The read answers, so the page is a real arrangement rather than the
      // empty panel the read-failure sentence is about.
      if (cmd === "topbar_items") return Promise.resolve(ITEMS);
      if (cmd === "config_get" || cmd === "config_get_default") return Promise.resolve({});
      // The one refusal this fixture is about.
      if (cmd === "config_set") return Promise.reject("Read-only file system (os error 30)");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: hide one item, which is a `setShown` and therefore a save.
(function () {
  var tries = 0;
  function tick() {
    var sw = document.querySelector('button[role="switch"], [role="switch"]');
    if (sw) { sw.click(); return; }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
