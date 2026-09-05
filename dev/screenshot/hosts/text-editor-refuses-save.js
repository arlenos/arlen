// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Konnte nicht speichern
//
// AND IT DOES NOT REACH THAT TODAY through `probe-host.sh`. Measured 6
// September: the page renders, `.cm-content` is present, the synthetic Ctrl+S
// below fires, and seven seconds later no refusal is anywhere in the page's
// text. `Mod-s` runs `onsave` unconditionally in `Buffer.svelte`, so the
// keymap is not the condition - the synthetic KeyboardEvent is not reaching
// CodeMirror's handler in this webview. The line above is what the state
// SAYS, kept so the check fails loudly rather than the fixture passing on a
// page that never refused anything; `shoot.sh`'s SHOOT_DRIVE path is the one
// that has driven this gesture, and it cannot install a host.
//
// A runtime where the document OPENS and the save is refused - a read-only file,
// the ordinary way a save fails. Neither preview can reach it: with no runtime
// there is no save to refuse, and with --stub-host the open fails first, so the
// page never has a document to try to save.
//
// `editor_save` answers with the tagged problem the host now returns, so this
// also checks the page turns that tag into a sentence rather than showing it.
(function () {
  var PATH = "/home/tim/notes.md";
  var TEXT = "# Notes\n\nThe save below is refused, which is the point.\n";

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "initial_file") return Promise.resolve(PATH);
      if (cmd === "editor_open") {
        return Promise.resolve({ path: PATH, text: TEXT, stamp: "1:2" });
      }
      if (cmd === "editor_save") {
        return Promise.reject({ problem: "unwritable", why: "Permission denied (os error 13)" });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Type something so the buffer is dirty, then press Save. Waits for the editor
  // rather than firing on a timer, so a slow first paint moves the shot instead
  // of producing an empty one that reads like a finding.
  // There is no Save BUTTON: the gesture is Ctrl+S, bound in CodeMirror's keymap
  // inside the buffer. So the press goes to the editor's own contenteditable,
  // which is what CodeMirror listens on.
  var tries = 0;
  function press() {
    var cm = document.querySelector(".cm-content");
    cm.focus();
    cm.dispatchEvent(new KeyboardEvent("keydown", {
      key: "s", code: "KeyS", keyCode: 83, which: 83,
      ctrlKey: true, bubbles: true, cancelable: true,
    }));
  }
  function tick() {
    var line = document.querySelector(".cm-line");
    if (line) {
      // TYPE FIRST, and the comment above has said so since this was written
      // while the code did not do it. `save()` returns on its first line when
      // `draft === null`, and `draft` is null until the buffer changes - so
      // every Ctrl+S here landed on a clean document and returned before it
      // could be refused. The fixture rendered a document and no refusal for
      // as long as it existed, which is what `// EXPECT:` now catches.
      //
      // A text node rather than a synthetic `input`: CodeMirror watches its own
      // content with a MutationObserver, so an edit to the DOM is an edit to the
      // document, and that is the one gesture a script can make that the editor
      // reads the same way as a keystroke.
      line.appendChild(document.createTextNode(" x"));
      setTimeout(press, 300);
      return;
    }
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 400); });
})();
