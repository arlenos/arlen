// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Weit über die Berge
//
// A runtime with a media player registered, so the now-playing applet exists.
//
// Last of the four applets that do not render on a plain fixture. `nowPlaying`
// being null hides the applet entirely, and the store's own comment says why that
// is right - an absent player is not a claim worth erroring over - which also
// means the surface is unreachable without a host.
//
// The answer is spelled out at the call site rather than referenced, because
// `check-fixture-answers-whole` reads the literal there to prove every field of
// the shape is present. It caught the sound fixture doing the other thing an hour
// ago, and its header is right: a fixture is evidence, and a partial one produces
// a wrong picture rather than a smaller one.
//
// TWO players, and the second is paused: the panel lists every registered player
// and marks the ones that are not the active one, so a fixture with a single
// entry cannot show that row at all. `canSeek` is false to keep the progress bar
// honest about a stream that cannot be scrubbed.
(function () {
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "mpris_now_playing") {
        return Promise.resolve({
          title: "Weit über die Berge",
          artist: "Die Sterne",
          album: "Posen",
          artUrl: null,
          status: "playing",
          position: 74,
          length: 212,
          canSeek: false,
          canPrev: true,
          canNext: true,
          canPause: true,
          canControl: true,
          players: [
            { id: "org.mpris.MediaPlayer2.vlc", app: "VLC", icon: null, status: "playing" },
            { id: "org.mpris.MediaPlayer2.firefox", app: "Firefox", icon: null, status: "paused" },
          ],
          activeId: "org.mpris.MediaPlayer2.vlc",
        });
      }
      if (cmd === "set_popover_input_region") return Promise.resolve(null);
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
