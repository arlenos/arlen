// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Aus dem Verlauf entfernen
//
// A runtime for the launcher with clipboard history behind it.
//
// This group had NO swept surface, which is why the violation in it went
// unreported for as long as it did: the per-entry remove button lived inside a
// `CommandItem`, an `option` inside a `listbox`, which may not contain a control.
// Nothing rendered the group, so no sweep ever looked at it. The fixture exists
// so the state that carried the fault is a state the sweep visits.
//
// It cannot be reached by hand here: the clipboard group needs the opt-in flag
// (`[clipboard] enabled` in shell.toml) AND the plugin bridge behind
// `waypointer_search_plugin`, so a launcher on a dev server shows nothing under
// any query. The fixture answers exactly those two and refuses everything else,
// the way the stub host does, so no other provider fills the list and the picture
// is about this group.
(function () {
  var ENTRIES = [
    {
      id: "clip-1",
      title: "1873 Hofsteigstraße, Wolfurt",
      description: "files - 48m",
      icon: null,
      relevance: 0.9,
      action: null,
      plugin_id: "core.clipboard",
    },
    {
      id: "clip-2",
      title: "https://arlen.dev/docs/clipboard-api",
      description: "files - 4m",
      icon: null,
      relevance: 0.6,
      action: null,
      plugin_id: "core.clipboard",
    },
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      if (cmd === "clipboard_is_enabled") return Promise.resolve(true);
      if (cmd === "waypointer_search_plugin" && args && args.pluginId === "core.clipboard") {
        return Promise.resolve(ENTRIES);
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "waypointer" }, currentWebview: { label: "waypointer" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: type a query so the group appears, then press ArrowDown until the
// selection is on a clipboard row, because the footer action names the row under
// the selection and an unhighlighted group would photograph as hints only.
(function () {
  var tries = 0;
  function typed() {
    var input = document.querySelector("input[cmdk-input], [cmdk-input], .wp-input-wrap input, input");
    if (!input) return false;
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    setter.call(input, "clipboard");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  function selected() {
    var row = document.querySelector('[data-slot="command-item"][data-selected]');
    return !!(row && (row.getAttribute("data-value") || "").indexOf("clip-item-") === 0);
  }
  function step() {
    var input = document.querySelector("input[cmdk-input], [cmdk-input], input");
    if (!input) return;
    input.focus();
    ["keydown", "keyup"].forEach(function (type) {
      input.dispatchEvent(new KeyboardEvent(type, {
        key: "ArrowDown", code: "ArrowDown", keyCode: 40, which: 40, bubbles: true,
      }));
    });
  }
  var stage = 0;
  function tick() {
    if (stage === 0 && typed()) stage = 1;
    else if (stage === 1) {
      if (selected()) return;
      step();
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
