// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Drück eine Tastenkombination
//
// A runtime for the shortcuts page with the KEY-CAPTURE dialog open.
//
// It was the last Settings dialog nothing drove, and the reason is worth keeping:
// under vite the binding read fails, so no row renders, so there is no pill to
// press. `/keyboard/shortcuts` has been a sweep row for days and what it
// photographs is the read-failure state - the honest one, but not the page a
// person with a working daemon sees, and not a route to this dialog.
//
// So the fixture answers the two reads and then presses a binding.
(function () {
  // Every field of `KeybindingEntry`, including the three the Rust side marks
  // `skip_serializing_if` - a fixture that answers less than the shape is how a
  // consumer throws mid-render and the page keeps whatever it was showing.
  function entry(over) {
    var e = {
      id: "focus_left",
      action: "focus_left",
      binding: "Super+H",
      default_binding: "Super+H",
      is_custom: false,
      category: "focus",
      label_key: null,
      label: "Focus left",
      description_key: null,
      module_id: null,
    };
    for (var k in over) e[k] = over[k];
    return e;
  }
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "keybindings_get_all") {
        return Promise.resolve([
          entry({}),
          entry({ id: "focus_right", action: "focus_right", binding: "Super+L", default_binding: "Super+L", label: "Focus right" }),
        ]);
      }
      if (cmd === "keybindings_get_conflicts") return Promise.resolve([]);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: expand the category the rows live in, then press a binding pill.
// The categories start collapsed, so a pill that exists in the store is not in
// the DOM until the section is open.
(function () {
  var tries = 0;
  var stage = 0;
  function tick() {
    if (stage === 0) {
      var cat = document.querySelector("#cat-focus button");
      if (cat) { cat.click(); stage = 1; }
    } else if (stage === 1) {
      var pill = document.querySelector(".kb-pill");
      if (pill) { pill.click(); return; }
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 200); });
})();
