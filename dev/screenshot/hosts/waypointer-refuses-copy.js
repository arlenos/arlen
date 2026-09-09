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
// simply works. So the fixture refuses it on purpose. It answers the launcher's
// inline evaluation so there is something copyable on screen, and makes
// `writeText` reject so pressing Enter on it fails.
//
// IT USED TO SERVE `search_unicode`, and that stopped working on 8 September
// when the unicode provider was removed by decision ("the keyword stays, the
// implementation does not"). The fixture went on driving a plugin that no longer
// exists - typing a query, waiting for a row that could never appear - and said
// nothing, because `probe-host.sh` was refusing every fixture in the tree for an
// unrelated reason until 10 September. The refusal itself is untouched by that
// ruling: an inline result still copies through `navigator.clipboard` and still
// sets `sh.wp.errCopy` when the write throws
// (`WaypointerContent.svelte`, `handleInlineAction`), so the fixture moved to the
// provider that is still there rather than being deleted with the one that went.
//
// What the picture has to show is the launcher STILL OPEN with the sentence in
// it. A toast would have been visible too - `raiseRefusal` reaches the top bar
// from here - but it would have taken the surface away, and a refusal you cannot
// retry from is only half an answer.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      // The whole shape the client reads: `result_type`, `display` and the
      // `copy_value` the Enter handler writes to the clipboard.
      if (cmd === "evaluate_waypointer_input") {
        return Promise.resolve({
          result_type: "math",
          display: "12 × 12 = 144",
          copy_value: "144",
        });
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

// Drive it: type a calculation into the launcher's input so the inline result
// appears, then press Enter, which is the gesture that copies it. Polls for each
// step rather than firing on a timer, so a slow first paint moves the shot
// instead of producing an empty one.
(function () {
  var tries = 0;
  function typed() {
    var input = document.querySelector("input[cmdk-input], [cmdk-input], .wp-input input, input");
    if (!input) return false;
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    setter.call(input, "12*12");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  function press() {
    // The inline result renders into `#wp-inline-result`, and pressing before it
    // is there presses on nothing - which is what produced a picture of an idle
    // launcher the first time this was written against a row.
    var shown = document.getElementById("wp-inline-result");
    if (!shown || !shown.textContent.trim()) return false;
    // Enter on the INPUT, not a click on the row. cmdk keeps the highlight
    // itself and runs `onSelect` from its own key handler; a synthetic click on
    // the element runs no handler at all, which is what the first cut of this
    // fixture did - it produced a picture of the row sitting there and read as
    // "the copy did not fail" when nothing had been pressed.
    var input = document.querySelector("input[cmdk-input], [cmdk-input], input");
    if (!input) return false;
    input.focus();
    // ON THE WINDOW AS WELL AS THE INPUT. The launcher's Enter handler is a
    // `<svelte:window onkeydown>`, and a synthetic event dispatched at the input
    // reaches it only if nothing in between stops it - which is a property of
    // whatever cmdk does that day, not of this fixture. Dispatching at both ends
    // is the same press either way and does not depend on that.
    ["keydown", "keypress", "keyup"].forEach(function (type) {
      var ev = new KeyboardEvent(type, {
        key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true,
      });
      input.dispatchEvent(ev);
      window.dispatchEvent(
        new KeyboardEvent(type, {
          key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true,
        }),
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
