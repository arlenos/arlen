// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// A runtime for the task manager where the list is REAL and the action is
// refused - the state neither --stub-host nor a bare vite preview can reach, and
// the one an ordinary person meets: End process on something they do not own.
//
// With no runtime the store serves its fixture and its own `if (tauriAvailable)`
// guard means an action can never be refused. With --stub-host the list refuses
// too, so the page has no row to press. Only here does the refusal sentence
// render, which is why it went unlooked-at for as long as it did.
//
// `stop_process` answers with the token the host now returns, not a sentence, so
// this also checks that the page can turn that word into one.
(function () {
  var ROWS = [
    { id: 4211, name: "chrome", group: "app", status: "running", cpu: 24.5, memMB: 1840, diskKBs: 12, netKBs: 340 },
    { id: 1, name: "systemd", group: "system", status: "running", cpu: 0.2, memMB: 12, diskKBs: 0, netKBs: 0, critical: true },
    { id: 812, name: "arlen-graph-daemon", group: "background", status: "running", cpu: 1.1, memMB: 96, diskKBs: 3, netKBs: 1 },
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      if (cmd === "list_app_rows" || cmd === "list_processes") {
        return Promise.resolve(ROWS);
      }
      if (cmd === "stop_process" || cmd === "freeze_process" || cmd === "limit_process") {
        return Promise.reject("not-permitted");
      }
      if (cmd === "renice_process") return Promise.reject("not-permitted");
      if (cmd === "process_nice") return Promise.resolve(0);
      if (cmd === "nice_levels") {
        return Promise.resolve([["Highest", -5], ["High", -2], ["Normal", 0], ["Low", 5], ["Lowest", 10]]);
      }
      // Everything unnamed refuses in a way no app decodes, same as the stub
      // host: the escape branch is worth seeing here too.
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive the surface to the state under test.
//
// Stop lives in a right-click menu, and `--open` left-clicks. Rather than teach
// the shot tool a second gesture for one surface, the host that installs the
// runtime also performs the interaction: wait for the rows the runtime above
// served, open the menu on the first one, press Stop. `--settle` then gives the
// refusal time to land before the picture is taken.
//
// It waits for a ROW rather than firing on a timer, so a slow first paint moves
// the shot rather than producing an empty one that looks like a finding.
(function () {
  var tries = 0;
  function press(el) {
    el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, clientX: 300, clientY: 300 }));
    setTimeout(function () {
      var items = Array.from(document.querySelectorAll('[role="menuitem"], .item, button'));
      var stop = items.find(function (b) {
        var s = (b.textContent || "").trim();
        return s === "Beenden" || s === "Stop" || s === "End process";
      });
      if (stop) stop.click();
    }, 250);
  }
  function tick() {
    var row = document.querySelector("tbody tr, .row");
    if (row) return press(row);
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 200); });
})();
