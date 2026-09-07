// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Beispiel-Hosts - nicht deine gespeicherten Verbindungen.
//
// The terminal's quick-connect palette, which no route and no single click
// reaches: it opens on Ctrl+Shift+R, or through two clicks in the sidebar's
// dropdown. A driver that presses the shortcut is the cheap way in.
//
// NO TAURI STUB HERE, deliberately. The palette's host lists ship fixture data,
// so under vite it has content of its own; installing a runtime would only
// change what the rest of the window does and put a second thing in the picture.
// A host script is allowed to be nothing but a driver.
(function () {
  var tries = 0;
  function tick() {
    // The layout listens on the window with `e.key.toLowerCase() === "r"`, so the
    // event needs a real `key` rather than only a keyCode - a synthetic event
    // with the wrong shape opens nothing and looks exactly like a palette that
    // is broken.
    var e = new KeyboardEvent("keydown", {
      key: "R",
      code: "KeyR",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    window.dispatchEvent(e);
    document.dispatchEvent(e);
    if (document.querySelector(".qc-card")) return;
    if (tries++ < 60) setTimeout(tick, 150);
  }
  window.addEventListener("load", function () { setTimeout(tick, 400); });
})();
