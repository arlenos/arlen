// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Kopfhörer
//
// A runtime with a sound server behind it, so the audio panel can be opened.
//
// It is the second of the four applets that do not render on a plain fixture at
// all - no sound server, so no applet, so no panel, so a click spec for it
// refuses every run and the surface stayed unmeasured. The battery panel was the
// first; `tray` and `mpris` are still owed one.
//
// The EXPECT is a device NAME rather than a label from our own catalogue: a
// catalogue string would prove the panel opened, and a name from this file proves
// the READ arrived. That distinction cost a run on the consent card, where the
// page showed its heading and no card.
//
// `set_popover_input_region` is answered because `openPopover` reverts without
// it - the shell is a layer surface whose region is the bar alone until that call
// widens it, so a panel drawn without one hands its clicks to the window behind.
// Any host that opens a panel owes this answer.
(function () {
  var STATE = {
    status: { volume: 62, muted: false, output_type: "bluetooth_headphones" },
    input_status: { volume: 45, muted: true },
    outputs: [
      { id: "bt-hp", name: "Kopfhörer (Bluetooth)", is_default: true },
      { id: "hdmi", name: "Bildschirm (HDMI)", is_default: false },
      { id: "line", name: "Analoger Ausgang", is_default: false },
    ],
    inputs: [
      { id: "mic", name: "Mikrofon (intern)", is_default: true },
      { id: "bt-mic", name: "Headset-Mikrofon", is_default: false },
    ],
    // Two apps, so the per-app section renders its rows rather than its empty
    // line: the volume figure on each is one of the three the percent key
    // changed this morning, and this is the only way to read them on screen.
    apps: [
      { id: 4711, name: "Musik", volume: 80, icon_data: null },
      { id: 4712, name: "Konferenz", volume: 35, icon_data: null },
    ],
  };

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // TWO READS, not one, and the first run found that out the hard way: the
      // APPLET asks `get_audio_status` and the PANEL asks `get_audio_full_state`,
      // so a host answering only the panel's read leaves the applet hidden and
      // the click has nothing to hit. The harness refused with "matched no
      // element", which is the right answer to a spec whose target is not there.
      // SPELLED OUT rather than `STATE.status`, and `check-fixture-answers-whole`
      // is why: it reads the literal at the answer site to prove every field of
      // the command's shape is there, and a member expression is not something it
      // can follow - it read the enclosing object and reported six fields against
      // three. A fixture is evidence, so being CHECKABLE is worth repeating three
      // values; they must stay equal to `STATE.status` below.
      if (cmd === "get_audio_status") {
        return Promise.resolve({
          volume: 62,
          muted: false,
          output_type: "bluetooth_headphones",
        });
      }
      if (cmd === "get_audio_full_state") return Promise.resolve(STATE);
      if (cmd === "set_popover_input_region") return Promise.resolve(null);
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
