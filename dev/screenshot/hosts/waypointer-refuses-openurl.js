// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Dieser Link ließ sich nicht öffnen
//
// A runtime for the launcher where a link will not open.
//
// Typing something that looks like a URL puts the launcher in url mode and Enter
// opens it. Until 10 September that path fired the call and closed the window on
// the same tick, so a link that would not open said nothing at all, on the one
// surface that had already gone.
//
// THE SECOND OF THE TWO SENTENCES, and that is the whole reason this exists
// beside `waypointer-refuses-run`. The four special-mode actions - run, man page,
// link, web search - produce exactly two messages between them: `sh.wp.errRun`
// for the two that run a command and `sh.wp.errOpenUrl` for the two that open
// something. The run fixture covers the first. This covers the second. Fixtures
// for the man page and the web search would drive the same helper to the same
// sentence, and a fixture that adds no distinct claim is a slower sweep for
// nothing.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "open_url") {
        return Promise.reject("no handler for https://example.invalid");
      }
      // Everything else refuses the way the stub host does, so no provider fills
      // the list and the shot is about the one sentence.
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "waypointer" }, currentWebview: { label: "waypointer" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: type the shell prefix and a command, then press Enter. Polls for the
// input rather than firing on a timer, so a slow first paint moves the shot
// instead of producing an empty one.
(function () {
  var tries = 0;
  function typed() {
    var input = document.querySelector("input[cmdk-input], [cmdk-input], input");
    if (!input) return false;
    var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
    setter.call(input, "example.invalid");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  function press() {
    // WAIT FOR THE MODE, not for a tick. URL detection runs in the query watcher,
    // which polls, and pressing before it has run presses in ordinary search
    // mode. The inline card carries the address once the mode is set.
    var shown = document.getElementById("wp-inline-result");
    if (!shown || shown.textContent.indexOf("example.invalid") === -1) return false;
    var input = document.querySelector("input[cmdk-input], [cmdk-input], input");
    if (!input) return false;
    input.focus();
    // ON THE WINDOW AS WELL AS THE INPUT, for the same reason the copy fixture
    // does it: the launcher's Enter handler is a `<svelte:window onkeydown>`,
    // and whether a synthetic event dispatched at the input reaches it is a
    // property of whatever cmdk does that day.
    ["keydown", "keypress", "keyup"].forEach(function (type) {
      var init = { key: "Enter", code: "Enter", keyCode: 13, which: 13, bubbles: true };
      input.dispatchEvent(new KeyboardEvent(type, init));
      window.dispatchEvent(new KeyboardEvent(type, init));
    });
    return true;
  }
  var stage = 0;
  function tick() {
    if (stage === 0 && typed()) { stage = 1; }
    else if (stage === 1 && press()) { return; }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
