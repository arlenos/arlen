// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// A runtime where the message OPENS and saving its attachment is refused - a
// full downloads folder, the ordinary way that write fails. Neither preview
// reaches it: with no runtime there is no message to open, and with
// --stub-host the read fails first, so there is no attachment chip to press.
//
// What the frame is for: `AttachmentRow` turns the tagged problem into a
// sentence and shows it in the same place as the SUCCESS line, distinguished
// only by a class. So the picture answers two things a reading cannot - whether
// the refusal is legible as a refusal rather than as a quieter success, and
// whether the filesystem's own words reach the reader inside it.
(function () {
  var PATH = "/home/tim/Mail/inbox/2026-08-26-invoice.eml";

  var MESSAGE = {
    from: "Renate Bucher <r.bucher@example.at>",
    subject: "Rechnung August",
    date: "Wed, 26 Aug 2026 08:12:04 +0200",
    text: "Hallo Tim,\n\nanbei die Rechnung fuer August.\n\nViele Gruesse\nRenate\n",
    to: ["tim@example.at"],
    cc: [],
    has_html: false,
    only_in_text: [],
    only_in_html: [],
    refusal: null,
    channels: [],
    attachments: [{ name: "rechnung-august.pdf", media_type: "application/pdf", bytes: 84213 }],
    sealed: null,
    invitation: null,
    path: PATH,
  };

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "launch_file") return Promise.resolve(PATH);
      if (cmd === "mail_read") return Promise.resolve(MESSAGE);
      if (cmd === "mail_save_attachment") {
        return Promise.reject({ problem: "not-written", why: "No space left on device (os error 28)" });
      }
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };

  // Press the attachment chip. Polls for it rather than firing on a timer: the
  // message has to be read and rendered first, and a shot taken early reads like
  // a finding when it is only an empty frame.
  var tries = 0;
  function tick() {
    var chip = document.querySelector("button.chip");
    if (chip) { chip.click(); return; }
    if (tries++ < 60) setTimeout(tick, 100);
  }
  window.addEventListener("load", function () { setTimeout(tick, 500); });
})();
