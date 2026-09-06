// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: siehe notes.md für die Abmachung
//
// The EXPECT is the BACKLINK this host serves, not the word the sample caption
// carries. "Beispiel" would have done as a string and proved nothing: it is a
// prefix of "Beispielkontext", the WHOLE-panel caption, so the fixture would
// have passed just as happily on the state it exists to distinguish itself from.
// A snippet only this host serves can only appear if `related_of` was reached -
// which under the old code it never was, because the refused provenance read
// threw first.
//
// A runtime where the lens reads TWO of its three sources and is refused the
// third, so one section of the panel is a sample and the rest is this file's own
// graph neighbourhood.
//
// It is the state the panel could not reach until 7 September. `provenance_of`
// sat bare inside the outer try while `related_of` and `project_of` each had
// their own, so a refused provenance read threw past both before they were
// attempted and dropped the WHOLE panel to its fixture - including the parts
// that would have answered. There was no per-section state to photograph.
//
// Neither of the other host states reaches it either: with no runtime nothing is
// asked, and with --stub-host all three refuse, which is the whole-panel caption
// and a different picture.
(function () {
  var PATH = "/home/tim/notes.md";
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "initial_file") return Promise.resolve(PATH);
      if (cmd === "editor_open") {
        return Promise.resolve({ path: PATH, text: "# Notes\n\nOne section of the lens is a sample.\n", stamp: "1:2" });
      }
      // The one refusal this fixture is about.
      if (cmd === "provenance_of") return Promise.reject("out-of-scope");
      if (cmd === "related_of") {
        return Promise.resolve([
          { file: "meeting.md", ref: "/home/tim/meeting.md", snippet: "siehe notes.md für die Abmachung" },
        ]);
      }
      if (cmd === "project_of") {
        return Promise.resolve({
          project: { name: "Arlen", members: [{ path: "/home/tim/plan.md", name: "plan.md" }] },
          unrecorded: false,
        });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
