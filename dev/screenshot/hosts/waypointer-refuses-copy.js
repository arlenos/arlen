// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Das ließ sich nicht kopieren
//
// A runtime for the launcher where a copy is refused.
//
// The launcher is the one surface that DISAPPEARS on the click that failed, so
// it is where a swallowed answer costs most: until today `copyUnicodeChar` fired
// the write, ignored it and hid the window in the same tick, and a refused
// clipboard left the person holding their previous clipboard with nothing said.
// They find out at the paste.
//
// It cannot be reached by hand, and not because of a backend: the clipboard
// simply works. So the fixture refuses it on purpose. It serves `search_unicode`
// so there is a row to press, and makes `writeText` reject so pressing it fails.
//
// What the picture has to show is the launcher STILL OPEN with the sentence in
// it. A toast would have been visible too - `raiseRefusal` reaches the top bar
// from here - but it would have taken the surface away, and a refusal you cannot
// retry from is only half an answer.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      if (cmd === "search_unicode") {
        return Promise.resolve([
          { codepoint: 8364, codepoint_hex: "U+20AC", name: "EURO SIGN", char_str: "€" },
        ]);
      }
      // Everything else refuses the way the stub host does, so no other provider
      // fills the list and the shot is about the one row.
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "waypointer" }, currentWebview: { label: "waypointer" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // The refusal itself. Defined rather than deleted, because a missing
  // `navigator.clipboard` takes a different branch in some browsers and this
  // fixture is about the REJECTED write, not the absent API.
  try {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: function () { return Promise.reject(new Error("denied")); } },
    });
  } catch (e) {
    /* If the property will not take, the drive below still runs and the probe
       says the state was not reached, which is the honest outcome. */
  }
})();

// Drive it: type into the launcher's input so the unicode provider runs, wait
// for the row, press it. Polls for each step rather than firing on a timer, so a
// slow first paint moves the shot instead of producing an empty one.
(function () {
  var tries = 0;
  function typed() {
    var input = document.querySelector("input[cmdk-input], [cmdk-input], .wp-input input, input");
    if (!input) return false;
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    setter.call(input, "unicode euro");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  function press() {
    var row = document.querySelector('[cmdk-item][data-value^="unicode-"], [data-value^="unicode-"]');
    if (!row) return false;
    // Enter on the INPUT, not a click on the row. cmdk keeps the highlight
    // itself and runs `onSelect` from its own key handler; a synthetic click on
    // the element runs no handler at all, which is what the first cut of this
    // fixture did - it produced a picture of the row sitting there and read as
    // "the copy did not fail" when nothing had been pressed.
    var input = document.querySelector("input[cmdk-input], [cmdk-input], input");
    if (!input) return false;
    input.focus();
    ["keydown", "keypress", "keyup"].forEach(function (type) {
      input.dispatchEvent(
        new KeyboardEvent(type, { key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true }),
      );
    });
    return true;
  }
  var stage = 0;
  function tick() {
    if (stage === 0 && typed()) stage = 1;
    else if (stage === 1 && press()) return;
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
