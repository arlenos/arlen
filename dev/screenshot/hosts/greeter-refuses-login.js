// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// A runtime where the login screen comes up with real profiles and the sign-in is
// refused with the token the host now returns. --stub-host alone loses the
// profile list, so there is nobody to sign in as and the refusal never renders.
(function () {
  var PROFILES = [
    { id: "tim", name: "Tim Kicker", avatar_url: null, kind: "standard", last_used: true, factors: [] },
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "greeter_profiles") return Promise.resolve(PROFILES);
      if (cmd === "greeter_sessions") return Promise.resolve([{ id: "arlen", name: "Arlen" }]);
      if (cmd === "greeter_wallpaper") return Promise.resolve(null);
      // The ordinary failure on a machine whose login service is not up.
      if (cmd === "greeter_authenticate" || cmd === "greeter_factor_begin") {
        return Promise.reject("no-greetd");
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Type a password and submit, so the refusal is on screen for the shot.
  var tries = 0;
  function tick() {
    var f = document.querySelector('input[type="password"], input');
    if (!f) { if (tries++ < 60) setTimeout(tick, 100); return; }
    f.focus();
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    setter.call(f, "wrong-password");
    f.dispatchEvent(new Event("input", { bubbles: true }));
    setTimeout(function () {
      f.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, bubbles: true }));
      var form = f.closest("form");
      if (form) form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
    }, 200);
  }
  window.addEventListener("load", function () { setTimeout(tick, 500); });
})();
