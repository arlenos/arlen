// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Die Farbe wurde nicht gespeichert
//
// A runtime where the calendars LOAD and recolouring one is refused - a
// read-only calendar directory, the ordinary way that write fails. Neither
// preview reaches it: with no runtime the sidebar has fixture calendars and no
// backend to refuse, and with --stub-host the list never loads, so there is no
// swatch to press.
//
// Chosen because this exact path was wrong once: the store's `colorRefusal`
// comment records that it used `String(e)` on a `#[serde(tag = "problem")]`
// enum, which is `[object Object]`, so the sentence ended with that rather than
// with a reason. The tag here carries a `why`, so the picture shows whether the
// filesystem's words reach the reader inside a translated sentence or as a tag.
(function () {
  var CALS = [
    { id: "personal", name: "Personal", color: "#7aa2f7" },
    { id: "work", name: "Work", color: "#9ece6a" },
  ];
  var AGENDA = {
    events: [],
    directory: "/home/tim/.local/share/arlen/calendars",
    directory_exists: true,
    files: 2,
    unreadable: 0,
    service_running: true,
  };

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "calendar_calendars") return Promise.resolve(CALS);
      if (cmd === "calendar_agenda") return Promise.resolve(AGENDA);
      if (cmd === "launch_file") return Promise.resolve(null);
      if (cmd === "calendar_set_color") {
        return Promise.reject({ problem: "not-written", why: "Read-only file system (os error 30)" });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Two presses: the calendar's colour dot opens a popover, a swatch inside it
  // commits. Polls for each rather than firing on a timer - the list has to load
  // and the popover has to mount, and a shot taken early reads like a finding
  // when it is only an empty frame.
  var tries = 0;
  function pressSwatch() {
    var sw = document.querySelector(".palette .swatch:not(.on)");
    if (sw) { sw.click(); return; }
    if (tries++ < 40) setTimeout(pressSwatch, 100);
  }
  function openPalette() {
    var dot = document.querySelector("button.dot");
    if (dot) { dot.click(); tries = 0; setTimeout(pressSwatch, 200); return; }
    if (tries++ < 60) setTimeout(openPalette, 100);
  }
  window.addEventListener("load", function () { setTimeout(openPalette, 500); });
})();
