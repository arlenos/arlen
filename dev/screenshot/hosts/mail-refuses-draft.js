// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Dieser Entwurf wurde nicht gespeichert
//
// A runtime for the mail reader where the mailbox is LIVE and saving a DRAFT is
// refused.
//
// REACHED THROUGH REPLY, not New Message. `mailboxComposes` is sample-only -
// `mail-app.md` rules Compose absent live because sending needs an account - so
// live there is no New button at all. Reply and Forward are not that ruling: they
// answer a message in front of you and land in the same drafts folder, so the
// draft write is reachable and this is the path a person actually takes to it.
// The first cut of this fixture looked for the New button, found none, and
// photographed an inbox with no composer in it.
//
// It is the costliest refusal in this app, because the thing at risk is what the
// person typed. Until 7 September a failed save took Discard's route: the
// composer closed and the text went with it, silently. The comment that put it
// on that route was guarding against the opposite mistake - reporting a draft as
// kept in a folder it never reached - and it was right about that and blind to
// this.
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
      if (cmd === "mail_draft_save") return Promise.reject("not-permitted");
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();



// Drive it: open the message, press Reply, type, press Save to Drafts.
(function () {
  var tries = 0;
  var stage = 0;
  function type(el, value) {
    var proto = el.tagName === "TEXTAREA" ? window.HTMLTextAreaElement : window.HTMLInputElement;
    Object.getOwnPropertyDescriptor(proto.prototype, "value").set.call(el, value);
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }
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
      var row = document.querySelector(".row");
      if (row) { row.click(); stage = 1; }
    } else if (stage === 1) {
      var reply = byName("Antworten");
      if (reply) { reply.click(); stage = 2; }
    } else if (stage === 2) {
      var body = document.getElementById("compose-body");
      if (body) { type(body, "Der Text, den niemand verlieren wollte."); stage = 3; }
    } else if (stage === 3) {
      var save = document.getElementById("compose-save-draft");
      if (save) { save.click(); return; }
    }
    if (tries++ < 140) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 400); });
})();
