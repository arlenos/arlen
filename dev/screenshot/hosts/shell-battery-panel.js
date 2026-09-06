// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: 62%
//
// A runtime with a battery in it, so the battery panel can be opened at all.
//
// Four of the bar's ten applets do not render on a plain fixture - there is no
// sound server, no battery, no tray client and no player behind it - so a sweep
// spec that clicks them refuses every run, and the four surfaces stayed
// unmeasured for that reason rather than because anyone judged them fine. This is
// the first of them: two commands and a small shape, which is all the applet needs
// to appear and its panel to fill.
//
// The percentage is the EXPECT because it comes from HERE. The panel's header
// would prove only that the panel opened; a number this file chose proves the
// read arrived, which is the difference between a photograph of the surface and a
// photograph of its empty state.
//
// `set_power_profile` refuses on purpose: the panel has a whole branch for a
// power-mode write that does not take (`bat-refused`, role=alert), and a fixture
// where every write succeeds cannot reach it. Reading is what this host is for;
// the refusal is the one thing worth having beside it.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "get_battery_status") {
        return Promise.resolve({
          percentage: 62,
          charging: false,
          time_remaining_minutes: 143,
        });
      }
      if (cmd === "get_power_profile") return Promise.resolve("balanced");
      // WITHOUT THIS THE PANEL REFUSES TO OPEN, and correctly. The shell is a
      // layer surface whose input region is the bar alone until this call widens
      // it, so a panel drawn without it hands every click to the window behind -
      // `openPopover` reverts rather than leave one on screen inviting clicks
      // that go somewhere else. The first run of this host clicked the applet and
      // photographed "That panel could not be opened." instead, which the
      // state check caught. Any host that opens a panel owes this answer.
      if (cmd === "set_popover_input_region") return Promise.resolve(null);
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
