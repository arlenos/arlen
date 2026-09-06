// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Diese Nachricht wurde nicht verschoben
//
// A runtime for the mail reader where the mailbox is LIVE and the archive is
// refused.
//
// It cannot be reached any other way, and the reason is in the store: the four
// writes only run when `mailboxState` is `live`, which means a real maildir
// answered `mail_folders` and `mail_list`. Under vite the mailbox is the sample
// and the writes are in-memory; with `--stub-host` the folder read refuses too,
// so the mailbox is `unreadable` and there is no message to press Archive on.
// Only a host that serves the reads and refuses the ONE write puts a live
// message on screen and then declines to move it.
//
// What the picture is for: until today those writes were correct on the axis
// this tree gates - nothing moves on the surface until the mailbox says it moved,
// so a failed archive never claimed to have archived - and silent on the other
// half. The message stayed where it was, which is the truth and is also exactly
// what a mis-click looks like.
(function () {
  var MSG = {
    id: "m1",
    folderId: "inbox",
    from: "Mara Winter",
    subject: "Rehearsal moved to Thursday",
    snippet: "Short version: the hall is free Thursday at seven, so we take it.",
    dateMs: Date.now() - 7200000,
    unread: true,
  };
  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd, args) {
      if (cmd === "mail_folders") {
        return Promise.resolve([
          { id: "inbox", kind: "inbox", unread: 1 },
          { id: "archive", kind: "archive", unread: 0 },
          { id: "trash", kind: "trash", unread: 0 },
        ]);
      }
      if (cmd === "mail_store") return Promise.resolve("/home/you/Maildir");
      if (cmd === "mail_list") {
        return Promise.resolve(args && args.folderId === "inbox" ? [MSG] : []);
      }
      if (cmd === "mail_open") {
        // EVERY field of `Message`, not the ones this fixture happens to care
        // about. An earlier cut left out `to`, `cc`, `attachments` and four more,
        // and the reading surface iterates them - so MessageView threw while
        // rendering, Svelte bailed, and the pane kept the "pick a message" it had
        // been showing. It looked exactly like a live-mailbox defect and was
        // reported as one for half an hour.
        return Promise.resolve({
          from: "Mara Winter",
          subject: "Rehearsal moved to Thursday",
          date: null,
          text: "The hall is free Thursday at seven, so we take it.",
          has_html: false,
          only_in_text: [],
          only_in_html: [],
          refusal: null,
          to: ["you@example.org"],
          cc: [],
          channels: [],
          attachments: [],
          invitation: null,
          sealed: null,
          path: "/home/you/Maildir/new/m1",
        });
      }
      // No message was passed on the command line, which is what a host answers
      // for a window somebody opened from the launcher. Served rather than left
      // to the catch-all below on purpose: a refused `launch_file` puts the
      // reader's OTHER note on screen ("could not find out which message it was
      // asked to open"), and a fixture that photographs two refusals at once
      // cannot be read as evidence about either.
      if (cmd === "launch_file") return Promise.resolve(null);
      // The one refusal this fixture is about.
      if (cmd === "mail_move") return Promise.reject("not-permitted");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();

// Drive it: open the message, then press Archive. Polls for each step rather
// than firing on a timer, so a slow first paint moves the shot instead of
// producing an empty one that reads as a finding.
(function () {
  var tries = 0;
  var stage = 0;
  function tick() {
    if (stage === 0) {
      var row = document.querySelector(".row");
      if (row) { row.click(); stage = 1; }
    } else if (stage === 1) {
      // By accessible name, because the header controls are icon-only. The
      // catalogue is German here, which is also what the EXPECT line reads.
      var btn = document.querySelector('[aria-label="Archivieren"], [title="Archivieren"]');
      if (btn) { btn.click(); return; }
    }
    if (tries++ < 90) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 300); });
})();
