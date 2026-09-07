// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Auf diesem System können keine Apps entfernt werden
//
// A runtime for one app's page where `settings_app_uninstall` answers
// `unavailable`.
//
// A sibling of `settings-uninstall-refused.js`, which carries the reasoning.
// One line differs: the token the command rejects with, and therefore which of
// the page's four sentences a person reads.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // The page's four reads. `null` and `[]` are honest answers, not
      // placeholders: an app with no declared schema, no recorded meta and no
      // grant renders as itself, and the head with the button is unconditional.
      if (cmd === "access_grants") return Promise.resolve([]);
      if (cmd === "app_settings_page") return Promise.resolve(null);
      if (cmd === "settings_app_meta") return Promise.resolve(null);
      if (cmd === "settings_app_general") return Promise.resolve(null);
      // The one refusal this fixture is about. A REJECTED OBJECT, not a string:
      // the page reads `e.kind`, so a fixture that rejects with a sentence would
      // land in the `failed` arm and photograph the wrong branch while looking
      // like it worked.
      if (cmd === "settings_app_uninstall") {
        return Promise.reject({ kind: "unavailable", detail: "no install daemon on the session bus" });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: press Uninstall, then confirm. By the button's own words, because
// neither carries a test id and the catalogue is German here - which is also
// what the EXPECT line reads.
(function () {
  function byText(words) {
    var all = document.querySelectorAll("button");
    for (var i = 0; i < all.length; i++) {
      if ((all[i].textContent || "").trim() === words) return all[i];
    }
    return null;
  }
  var tries = 0;
  var stage = 0;
  function tick() {
    if (stage === 0) {
      var open = byText("Deinstallieren");
      if (open) { open.click(); stage = 1; }
    } else if (stage === 1) {
      var confirm = byText("Entfernen");
      if (confirm) { confirm.click(); return; }
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
