// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Home
//
// TWO TABS, which no route reaches. `TabStrip` renders only when `$tabs.length
// > 1`, so every files surface in every sweep has drawn a single-tab window and
// the whole strip - its tablist, its tab buttons, its per-tab close control -
// has never been measured by anything. That is the same gap the sweep's own
// header warns about one level up: what is not in the table is invisible rather
// than clean.
//
// NO TAURI STUB. The app opens a first tab on its own and the strip is pure
// frontend state; installing a runtime would change what the pane below draws
// and put a second thing in the picture. A host script is allowed to be nothing
// but a driver.
//
// Ctrl+T is the app's own shortcut (`+layout.svelte`), pressed rather than
// reaching into the store, so this drives the path a person drives. The handler
// listens on the window and reads `e.key`, so the event needs a real `key`
// rather than only a keyCode - a synthetic event with the wrong shape opens
// nothing and looks exactly like a strip that is broken.
(function () {
  var tries = 0;
  function tick() {
    var e = new KeyboardEvent("keydown", {
      key: "t",
      code: "KeyT",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    window.dispatchEvent(e);
    // The strip appears one tick after the store updates. Retry rather than
    // sleep: the first press can land before the app has opened its own first
    // tab, and then there is nothing to be the second one.
    if (!document.querySelector(".tab-strip") && ++tries < 40) {
      setTimeout(tick, 150);
    }
  }
  setTimeout(tick, 300);
})();
