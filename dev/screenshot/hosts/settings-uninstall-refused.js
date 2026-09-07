// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Der Installationsdienst hat abgelehnt
//
// A runtime for one app's page where `settings_app_uninstall` REFUSES.
//
// Why a host and not a route: uninstalling is the one destructive control this
// page has, and until 8 September the button beside it was disabled under a
// stale comment. The command answers a token now - `unavailable`, `refused`,
// `failed`, `unknown` - and the page writes one sentence per kind. Four
// branches, and nothing had ever driven any of them, which is the shape this
// tree keeps finding: a sentence a person only meets when something has already
// gone wrong is exactly the sentence nobody looks at.
//
// The daemon cannot be made to refuse from a route, so each kind gets a fixture
// that serves the page's four reads and rejects the fifth with that token. The
// three siblings differ in one line and say so.
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
        return Promise.reject({ kind: "refused", detail: "arlen-settings is part of the running desktop" });
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
