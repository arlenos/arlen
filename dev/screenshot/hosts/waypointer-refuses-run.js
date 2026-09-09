// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Das ließ sich nicht ausführen
//
// A runtime for the launcher where a shell command will not start.
//
// `>` in the launcher runs a command. Until 10 September that path fired the
// call and closed the window on the same tick - the helper returned nothing and
// caught nothing - so a command that could not start said nothing at all, on the
// one surface that had already gone. The person finds out by the thing not
// happening. The module-result branch thirty lines below had awaited its own
// calls since it was written and says why in its own comment: a toast renders in
// THIS window, which is the one being hidden, so a failure has to keep the
// launcher open and say it there.
//
// So this fixture is the control for that fix. It refuses
// `execute_shell_command`, types `> nosuchcommand`, presses Enter, and the
// picture has to show the launcher STILL OPEN with the sentence in it. Before the
// fix the same drive produced a shot of a closed launcher and no words anywhere.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "execute_shell_command") {
        return Promise.reject("no such command: nosuchcommand");
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
    setter.call(input, "> nosuchcommand");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    return true;
  }
  function press() {
    // WAIT FOR THE MODE, not for a tick. The `>` prefix is parsed in the query
    // watcher, which polls, and pressing before it has run presses in ordinary
    // search mode - which is what the first cut of this did: the shot showed the
    // shell hint and no refusal, because Enter had gone somewhere else a moment
    // earlier. The inline card carries the command once the mode is set.
    var shown = document.getElementById("wp-inline-result");
    if (!shown || shown.textContent.indexOf("nosuchcommand") === -1) return false;
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
