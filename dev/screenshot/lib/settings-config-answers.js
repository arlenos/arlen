// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// A runtime for Settings whose config READS answer and nothing else does.
//
// Not a fixture in `hosts/`: it reaches no refusal and drives no gesture. It is
// what `sweep-search-anchors.sh` renders under, because a config-backed page
// with no host draws one sentence where its rows would be, and every anchor on
// it would read as dead when the page is simply not there.
//
// The shortcut list answers one binding in each of two categories, because the
// compositor always has built-in bindings there and the page draws a category
// only when it holds one. The two are the categories the search points at; a
// third would need its line here, and the sweep says so by reporting it.
//
// The config answers an EMPTY file on purpose. Each page then draws its defaults, which
// is the state of a fresh install and the one a search has to work in. A row
// that only exists once a key is set, or once a daemon answered, stays absent
// here, and an anchor on such a row is rightly reported.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "config_get" || cmd === "config_get_default") return Promise.resolve({});
      if (cmd === "keybindings_get_conflicts") return Promise.resolve([]);
      if (cmd === "keybindings_get_all") {
        return Promise.resolve(["workspace", "tiling"].map(function (category) {
          return {
            id: category + ".probe", action: category + ".probe", binding: "Super+F12",
            default_binding: "Super+F12", is_custom: false, category: category,
            label_key: null, label: category, description_key: null, module_id: null,
          };
        }));
      }
      return Promise.reject("no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
