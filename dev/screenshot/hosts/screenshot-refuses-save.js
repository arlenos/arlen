// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Dieser Screenshot wurde nicht gespeichert
//
// A runtime where the capture SUCCEEDS and writing it to disk is refused - a
// full screenshots directory, the ordinary way a save fails. Neither preview
// reaches it: with no runtime there is nothing to save, and with --stub-host the
// capture fails first, so the app never holds a picture to try to keep.
//
// The path under test is DISMISS, not the Save button. Dismissing the floating
// thumbnail used to save, say "Saved to Pictures/Screenshots." and close the
// window 2.5 seconds later, all three unconditionally - so a refused write was
// announced as a successful one and then the capture was gone. The window should
// now stay, on the surface that can say what happened and still holds the image.
(function () {
  // A 2x2 PNG. Enough to be a real bitmap the canvas can load.
  var PNG =
    "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAFklEQVR4nGP8z8" +
    "DwnwEJMKEKMDAwMAAAJgAF/aAA1wAAAABJRU5ErkJggg==";

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // The GATE before any capture: `capturePrimary` asks this first and treats
      // a rejection as "the capture was refused". The first cut of this script
      // left it out, and the shot came back saying the screen could not be
      // photographed - a finding against the app that was really a hole in the
      // stub. Worth the line of comment: the picture looked like a defect.
      if (cmd === "capture_available") return Promise.resolve(true);
      if (cmd === "list_outputs") return Promise.resolve([{ name: "eDP-1", width: 2, height: 2 }]);
      if (cmd === "list_windows") return Promise.resolve([]);
      if (cmd === "capture_output" || cmd === "capture_window") {
        return Promise.resolve("data:image/png;base64," + PNG);
      }
      if (cmd === "save_screenshot") {
        return Promise.reject("No space left on device (os error 28)");
      }
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Press the thumbnail's dismiss. Polls for it rather than firing on a timer:
  // the capture has to land and decode first, and a shot taken early reads like
  // a finding when it is only an empty frame.
  var tries = 0;
  function tick() {
    var btns = document.querySelectorAll("button");
    for (var i = 0; i < btns.length; i++) {
      var label = (btns[i].getAttribute("aria-label") || "") + " " + (btns[i].title || "");
      if (/dismiss|verwerfen|schließen|close/i.test(label)) {
        btns[i].click();
        return;
      }
    }
    if (tries++ < 80) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 600); });
})();
