// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: konnte nicht geöffnet werden
//
// Deliberately the invariant half of the sentence. The line names the FILE now -
// "urlaub.jpg konnte nicht geöffnet werden: ..." - and the name and the reason
// each sit inside bidi isolates, so any EXPECT crossing one of those boundaries
// matches nothing and reads as a fixture that stopped working. This one crosses
// none. Writing the whole sentence out was the first attempt and it failed
// exactly that way, one minute after the wording changed.
//
// A runtime where the viewer is handed a real image and the decode is refused -
// a truncated download, a file whose extension lies, a format this build was not
// compiled with.
//
// The viewer had no fixture at all until now, which is why this exists: it is
// one of the few surfaces where a refusal REPLACES the whole view rather than
// sitting beside it, so what the window contains afterwards is the only thing a
// person has. The branch also carries the window controls, because a frameless
// window whose view failed to render is the one state where somebody most wants
// the close button and it used to have none.
//
// Neither preview state reaches it: with no runtime the app takes its demo path
// and shows a fixture picture, and `--stub-host` fails at `initial_file` before
// there is a file to decode.
(function () {
  var PATH = "/home/you/Bilder/urlaub.jpg";
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "initial_file") return Promise.resolve(PATH);
      if (cmd === "detect_media_kind") return Promise.resolve("image");
      // The one refusal this fixture is about. A tagged, human sentence rather
      // than a Rust error string: the surface decides between its own wording and
      // quoting the reason, and quoting is only right when the reason reads as
      // something a person wrote.
      if (cmd === "decode_image") {
        return Promise.reject("Die Datei bricht nach dem Kopf ab.");
      }
      if (cmd === "folder_position") return Promise.resolve([1, 3]);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
