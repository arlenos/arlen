// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: konnte nicht pausiert werden
//
// A runtime where the timeline READS and pausing it is refused.
//
// This is the most consequential switch in the app and the store says why in its
// own words: leaving it flipped after a failed write "would tell someone their
// activity is no longer being recorded while it is - which is the one thing this
// control must never do". So the picture has to show the switch back where it
// was AND a sentence, and neither preview state can produce it - with no runtime
// the write short-circuits before it is attempted, and with `--stub-host` the
// timeline itself does not read, so there is nothing to pause.
(function () {
  var NOW = Math.floor(Date.now() / 1000);
  function ev(id, verb, object, at) {
    // `kind` here is the ITEM's discriminant - "event" or a session - not the
      // event's own kind, which is the field one line down. Writing the event's
      // kind in both places made the store reject the whole read and the page
      // said the timeline could not be read at all, which is a different
      // sentence from the one this fixture is about.
    return {
      kind: "event",
      event: {
        id: id,
        kind: "file",
        verb: verb,
        object: object,
        source: "files",
        at: at,
      },
    };
  }
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "knowledge_timeline") {
        return Promise.resolve([
          ev("e1", "k.tl.verb.opened", "kapitel-3.md", NOW - 900),
          ev("e2", "k.tl.verb.opened", "urlaub.jpg", NOW - 5400),
        ]);
      }
      if (cmd === "knowledge_timeline_paused") return Promise.resolve(false);
      if (cmd === "knowledge_refresh_interval_ms") return Promise.resolve(30000);
      if (cmd === "knowledge_list" || cmd === "knowledge_searches") return Promise.resolve([]);
      // The one refusal this fixture is about.
      if (cmd === "knowledge_timeline_pause") return Promise.reject("not-permitted");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: press Pause. It is a plain BUTTON with its word in it, not a switch -
// the first cut of this looked for `[role="switch"]`, found nothing, and produced
// a photograph of a timeline with no refusal in it, which reads as "the write was
// accepted" rather than "nobody pressed anything".
(function () {
  var tries = 0;
  // BY ITS CLASS. This used to look for a button whose label starts with
  // `Pausieren`, which is a sentence the catalogue owns and would have stopped
  // matching the day somebody wrote a better word - the way five sibling
  // fixtures did on 10 September, silently. `button.tl-pause` is the timeline's
  // own control (`TimelineView.svelte`) and there is exactly one.
  function tick() {
    var hit = document.querySelector("button.tl-pause");
    if (hit) { hit.click(); return; }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 400); });
})();
