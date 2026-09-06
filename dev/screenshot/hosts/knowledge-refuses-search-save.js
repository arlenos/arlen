// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Nicht gespeichert
//
// A runtime where a search RUNS and saving it is refused.
//
// The store already put its optimistic row back on a refusal and had a good
// reason - a search that was not saved is gone at the next start and the person
// goes looking for it. What it did not do is SAY anything, and the caller cleared
// the name field and closed the naming row either way, so the whole gesture came
// back as if nobody had made it. The picture has to show the typed name still
// there, the row still open, and the sentence.
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
      // A result, because the save control lives in the results meta row and
      // does not exist until the view has state. The first cut answered with an
      // empty list and photographed a Searches page with no save button on it.
      if (cmd === "knowledge_search") {
        return Promise.resolve([
          { id: "r1", type: "file", title: "kapitel-3.md", sub: "files", at: Math.floor(Date.now() / 1000) - 600 },
        ]);
      }
      if (cmd === "knowledge_search_save") return Promise.reject("not-permitted");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();


// Drive it: go to Searches, press Save, type a name, confirm.
(function () {
  var tries = 0;
  var stage = 0;
  function byName(needle) {
    var hit = null;
    document.querySelectorAll("button").forEach(function (b) {
      var n = (b.getAttribute("aria-label") || b.textContent || "").trim();
      if (!hit && n.indexOf(needle) >= 0) hit = b;
    });
    return hit;
  }
  function tick() {
    if (stage === 0) {
      var nav = byName("Suchen");
      if (nav) { nav.click(); stage = 1; }
    } else if (stage === 1) {
      var q = document.querySelector('input[type="search"], .se-query input, input');
      if (q) {
        Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set
          .call(q, "kapitel");
        q.dispatchEvent(new Event("input", { bubbles: true }));
        stage = 15;
      }
    } else if (stage === 15) {
      var save = document.querySelector("button.se-save");
      if (save) { save.click(); stage = 2; }
    } else if (stage === 2) {
      var input = document.querySelector("input.se-save-name");
      if (input) {
        Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set
          .call(input, "Kapitel drei");
        input.dispatchEvent(new Event("input", { bubbles: true }));
        stage = 3;
      }
    } else if (stage === 3) {
      var confirm = document.querySelector("button.se-save");
      if (confirm) { confirm.click(); return; }
    }
    if (tries++ < 140) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 500); });
})();
