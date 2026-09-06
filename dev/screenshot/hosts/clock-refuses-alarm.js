// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Das hat den Uhrendienst nicht erreicht
//
// A runtime for the clock where the state is REAL and the write is refused - a
// person arming an alarm on a machine whose clock daemon has stopped answering.
//
// Neither of the other two host states reaches it. With no runtime the store
// serves its fixture and `send` returns early on `!tauriAvailable`, so an action
// can never fail; with `--stub-host` the READ refuses too, so the page shows the
// unavailable sentence and has no switch to press. Only here does the alarm
// exist, get pressed, and come back.
//
// Worth photographing for what it says as much as that it says anything: the
// store patches the alarm on, sends, and on refusal puts the state BACK. So the
// switch a person just moved snaps to where it was, and the sentence has to
// explain that rather than leave it looking like a glitch. That is the greeter's
// shape - the non-event and the state that now holds - and this is the picture
// that shows whether the clock keeps it.
(function () {
  var now = Date.now();
  var STATE = {
    wake_capable: true,
    alarms: [
      { id: "a1", time: "07:00", label: "", days: [0, 1, 2, 3, 4], enabled: true, fire_late: false,
        next_fire_at: now + 9 * 3600000 },
      { id: "a2", time: "09:30", label: "", days: [], enabled: false, fire_late: true, next_fire_at: null },
    ],
    timers: [],
    focus: null,
    focus_config: { focus_min: 25, break_min: 5, rounds: 4 },
    stopwatch: { running: false, started_at: null, accumulated_ms: 0, laps: [] },
    world: [],
  };

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "clock_state") return Promise.resolve(STATE);
      // The write a person presses. The daemon is there and says no.
      if (cmd === "clock_set_alarm" || cmd === "clock_toggle_alarm") {
        return Promise.reject("not-permitted");
      }
      // Everything unnamed refuses the way the stub host does, so the escape
      // branch is visible here too.
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive the surface: wait for the alarm rows the runtime above served, then
// press the switch on the second one - the one that is OFF, so the press is an
// arming rather than a disarming, which is the case a person cares about.
//
// It waits for a row rather than firing on a timer: a slow first paint should
// move the shot, not produce an empty one that reads as a finding.
(function () {
  var tries = 0;
  function tick() {
    var switches = document.querySelectorAll('[role="switch"], button[aria-checked]');
    if (switches.length >= 2) {
      switches[1].click();
      return;
    }
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 200); });
})();
