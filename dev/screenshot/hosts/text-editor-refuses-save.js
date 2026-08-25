// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
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
  function tick() {
    var cm = document.querySelector(".cm-content");
    if (cm) {
      cm.focus();
      cm.dispatchEvent(new KeyboardEvent("keydown", {
        key: "s", code: "KeyS", keyCode: 83, which: 83,
        ctrlKey: true, bubbles: true, cancelable: true,
      }));
      return;
    }
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 400); });
})();
