// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: 15,8
//
// A runtime that reports a machine under load, so the task manager's NUMBERS can
// be rendered at all.
//
// It exists because of a gap found on 6 September while checking a locale fix:
// under plain vite the rate counters never become ready, so every figure on the
// Performance tab is an em-dash and the CPU column in the process table is a
// hyphen. The app draws its own frame perfectly and says nothing a person would
// read as a measurement - which means the one thing that tab is FOR could not be
// photographed, and a change to how those numbers are written could not be
// verified on screen.
//
// `ratesReady` is the whole trick: the store carries it forward from the tick, and
// the surfaces key their "measured" versus "not measured" wording off it. A
// fixture that leaves it false is the state the app is already in without a host.
//
// The EXPECT is a German-formatted figure on purpose. It proves both halves at
// once: that a tick arrived, and that the number went through the locale rather
// than through `toFixed`, which wrote a decimal point in every language until the
// same day.
//
// It is the DISK figure and not the CPU one, and the first run is why: the CPU
// headline rounds to whole percent, so `37.4` renders as `37` and the check
// refused a page that was in fact perfectly correct. Disk is 12.5 + 3.25 read and
// written, printed to one decimal - the only figure here that has a decimal mark
// to be wrong about.
(function () {
  function cores(n) {
    var out = [];
    for (var i = 0; i < n; i++) {
      out.push({ user: 20 + ((i * 7) % 50), system: 4 + (i % 5), iowait: i % 3 });
    }
    return out;
  }

  var TICK = {
    cpuPct: 37.4,
    cpuCount: 8,
    memPct: 61.8,
    memUsedGb: 9.9,
    memTotalGb: 16,
    diskReadMbs: 12.5,
    diskWriteMbs: 3.25,
    netRxMbs: 1.75,
    netTxMbs: 0.5,
    ratesReady: true,
    memPressure: { some10: 4.5, full10: 0.5, level: "ok" },
    cores: cores(8),
    load: { one: 1.25, five: 0.95, fifteen: 0.7, perCore: 0.16 },
    devices: [{ name: "nvme0n1", readMbs: 12.5, writeMbs: 3.25 }],
    links: [{ name: "wlan0", rxMbs: 1.75, txMbs: 0.5 }],
    cpuTempC: { celsius: 54, label: "Package id 0" },
    coreFreqs: [3200, 3100, 2900, 3000, 3300, 2800, 3150, 3050],
  };

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "system_tick") return Promise.resolve(TICK);
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
