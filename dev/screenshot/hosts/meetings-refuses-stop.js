// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Die Aufnahme wurde nicht beendet
//
// A runtime for the meetings capture surface where the recording STARTS and the
// stop is refused - the state whose sentence is the most consequential one in
// this app, because a refused stop may leave the microphone live.
//
// The page's own comment says this state "cannot be reached by hand": it needs a
// capture that started, so a backend, and then a stop that failed, so no
// backend, and no single session gives you both. It carries a dev-only
// `?state=stop-failed` pin for that reason. A host fixture gives you both
// honestly - start resolves, stop rejects - so what gets photographed here is
// the REAL path rather than the pinned one, and the pin stays what it is: a way
// to look at the surface, not the only way it is ever seen.
//
// The one thing this picture has to show is the retry. A refused START leaves
// nothing running and a person can walk away; a refused STOP owes them another
// way to turn the microphone off, and an honest sentence without that button is
// a dead end.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "meetings_list") {
        return Promise.resolve({ state: "rows", rows: [] });
      }
      // The recording starts, so the red dot and the clock are a true claim.
      if (cmd === "meeting_start_capture") return Promise.resolve(null);
      // And the daemon will not let go of the microphone.
      if (cmd === "meeting_stop_capture") return Promise.reject("not-permitted");
      // Everything unnamed refuses the way the stub host does, so the escape
      // branch is visible here too.
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive the surface: wait for the Stop the running capture put on screen, press
// it, and let the refusal land. It waits for the button rather than firing on a
// timer, so a slow first paint moves the shot instead of producing an empty one
// that reads as a finding.
(function () {
  var tries = 0;
  function tick() {
    var stop = document.getElementById("stop");
    if (stop) return stop.click();
    if (tries++ < 80) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 200); });
})();
