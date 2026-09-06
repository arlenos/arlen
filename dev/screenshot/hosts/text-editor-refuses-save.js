// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Konnte nicht speichern
//
// IT REACHES IT NOW. This block used to say the opposite at length - that the
// synthetic Ctrl+S never got to CodeMirror's handler in this webview and that no
// refusal appeared seven seconds later, measured 6 September - and it was kept as
// a deliberate loud failure. Re-measured 7 September through the same
// `probe-host.sh`: the page reads "Konnte nicht speichern: diese Datei oder ihr
// Ordner ließ sich nicht beschreiben." and the probe exits 0.
//
// What changed is not recorded here because I do not know it, and guessing would
// put a second wrong sentence where the first one was. What IS worth keeping is
// why the old one survived a day: a comment that describes a state which has
// since ended reads exactly like a verdict about today.
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
