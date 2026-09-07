// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: arlen@hos
//
// The expected text is the INACTIVE workspace's window card, deliberately.
// The top bar already shows the active window's title, so expecting that
// would pass on a page where the overlay never opened - which is the exact
// failure `probe-host.sh` exists to catch.
//
// The workspace overlay (Super+Tab), which no route and no click reaches: it
// opens when the compositor emits `arlen://workspace-overlay-open`, and under
// vite there is no compositor.
//
// SO THIS STUB ALSO SPEAKS THE EVENT PLUGIN, which the other host fixtures do
// not need. `listen()` from `@tauri-apps/api/event` is not a separate API - it
// calls `invoke("plugin:event|listen", {event, handler})`, and `handler` is
// whatever `transformCallback` returned. These fixtures return the callback
// itself, so the registration can be caught and called later. That is the whole
// mechanism, and it makes every event-opened surface reachable, not just this
// one.
(function () {
  var listeners = {};

  function emit(name, payload) {
    (listeners[name] || []).forEach(function (h) {
      h({ event: name, id: 0, payload: payload });
    });
  }

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      if (cmd === "plugin:event|listen") {
        var key = args && args.event;
        (listeners[key] = listeners[key] || []).push(args.handler);
        // The real plugin answers an unlisten id.
        return Promise.resolve(1);
      }
      if (cmd === "plugin:event|unlisten") return Promise.resolve();
      if (cmd === "get_workspaces") {
        return Promise.resolve([
          { id: "w1", group_id: "g1", name: "1", active: false, output_connectors: ["DP-1"] },
          { id: "w2", group_id: "g1", name: "2", active: true, output_connectors: ["DP-1"] },
        ]);
      }
      if (cmd === "get_windows") {
        return Promise.resolve([
          {
            id: "win1",
            app_id: "dev.arlen.files",
            title: "Dokumente",
            active: true,
            minimized: false,
            fullscreen: false,
            workspace_ids: ["w2"],
            output_connectors: ["DP-1"],
            connector: "DP-1",
          },
          {
            id: "win2",
            app_id: "dev.arlen.terminal",
            title: "arlen@host: ~",
            active: false,
            minimized: false,
            fullscreen: false,
            workspace_ids: ["w1"],
            output_connectors: ["DP-1"],
            connector: "DP-1",
          },
        ]);
      }
      // The overlay CLOSES ITSELF if this refuses, and correctly so: without the
      // widened region it is drawn and every click inside falls through to the
      // window behind. The first cut of this fixture left it unanswered and got
      // a picture of "Das Panel liess sich nicht oeffnen" instead of an overlay,
      // which is the refusal working rather than the fixture failing.
      if (cmd === "set_popover_input_region") return Promise.resolve(true);
      // The bar's other two reads, answered so the shot holds ONE thing. Left
      // refusing, they put "focus mode could not be restored" and "your Quick
      // Settings layout could not be read" over the picture, and a shot with
      // three refusals in it is evidence about none of them.
      if (cmd === "get_focus_state") return Promise.resolve(null);
      // `tile`, not `tiles` - the Rust side renames it, and `check-fixture-answers-whole`
      // reads the WIRE name. An empty layout is what a machine with no saved order has.
      if (cmd === "qs_layout_get") return Promise.resolve({ tile: [] });
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Fire once the shell has registered its listener. Emitting before that is a
  // no-op that looks exactly like an overlay which refuses to open.
  var tries = 0;
  function tick() {
    if (listeners["arlen://workspace-overlay-open"]) {
      emit("arlen://workspace-overlay-open", null);
      return;
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 200); });
})();
