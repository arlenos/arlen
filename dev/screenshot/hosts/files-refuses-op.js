// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// A runtime where the folder LISTS and the operation is refused - the state a
// person meets when they press Delete on something they may not touch. Neither
// preview reaches it: with no runtime there is no backend to refuse, and with
// --stub-host the listing fails first so there is no row to act on.
//
// `files_op` answers with the tagged problem the host now returns, so this checks
// the red bar turns that tag into a sentence rather than showing it.
(function () {
  function entry(name, kind, size) {
    return {
      name: name,
      kind: kind,
      size: size,
      modified_unix: 1756000000,
      is_hidden: false,
      readonly: false,
      symlink_target: null,
      full_path: null,
    };
  }
  var ROWS = [
    entry("Documents", "dir", null),
    entry("notes.md", "file", 1840),
    entry("photo.jpg", "file", 220144),
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "files_list") return Promise.resolve(ROWS);
      if (cmd === "files_op") {
        // What the filesystem says most often on a delete: EACCES, which used to
        // reach the bar as "Permission denied (os error 13)".
        return Promise.reject({ problem: "io", why: "Permission denied (os error 13)" });
      }
      if (cmd === "files_bookmarks" || cmd === "files_devices" || cmd === "files_projects") {
        return Promise.resolve([]);
      }
      if (cmd === "files_saved_searches" || cmd === "files_smart_folders" || cmd === "files_templates") {
        return Promise.resolve([]);
      }
      if (cmd === "shell_present") return Promise.resolve(false);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it to the refusal. New folder is the cheapest op to reach - it needs no
// selection - and it lives in the background context menu, so the press is a
// right-click followed by the item. `--open` left-clicks only, which is why the
// host performs the gesture.
(function () {
  var tries = 0;
  function press(target) {
    target.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 500, clientY: 500 }));
    setTimeout(function () {
      var item = Array.from(document.querySelectorAll('[role="menuitem"], [data-menu-item]')).find(
        function (b) {
          var s = (b.textContent || "").trim();
          return s === "Neuer Ordner" || s === "New folder" || s === "New Folder";
        },
      );
      if (item) item.click();
    }, 300);
  }
  function tick() {
    // THE TRIGGER, not the pane around it. This dispatched `contextmenu` on
    // `main` for weeks, and events bubble UP: `main` is an ANCESTOR of the menu's
    // trigger, so the trigger never heard it, no menu opened, and the shot this
    // fixture exists to take showed a plain listing. Measured 6 September - the
    // page rendered with rows and no menu item anywhere - which is a fixture
    // documenting a state it does not produce.
    var pane = document.querySelector("[data-context-menu-trigger], .fm-browser, .browser");
    if (pane) return press(pane);
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 600); });
})();
