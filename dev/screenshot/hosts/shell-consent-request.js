// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: Zulassen?
//
// The card's own question, and class-agnostic on purpose: every request wears
// "<App> möchte <ask>. Zulassen?", so this one word proves the CARD drew rather
// than the state below it. That distinction is the whole reason for the check -
// the first render of this surface produced a toast reading "Diese Abfrage
// konnte nicht bedienbar gemacht werden", which is a page with no card on it and
// would pass any looser test.
//
//   dev/screenshot/probe-host.sh shell-consent-request http://localhost:6931/consent \
//     dev/screenshot/clipped-text.js 1280
//
// That entry point carries no query, so it reads the default request; address a
// specific class through headless.sh with `?consent=<n>`.
//
// A runtime where the consent broker HAS a request pending and the shell can
// make its surface answerable. It exists because the one surface in this system
// where a wrong sentence costs the most had never been rendered by this harness
// at all, and three separate gates each explain why:
//
//   - the card mounts only on the window whose Tauri label is `consent`, and
//     with no host the label reads `main`;
//   - the store's fixture queue is served only when there is NO runtime, because
//     a broker failure on a real boot must show nothing rather than cover the
//     desktop with an invented request - so `--stub-host` shows an empty window,
//     correctly;
//   - and the card refuses to draw at all unless `set_consent_input_region`
//     takes. That refusal is the right one (a card whose buttons hand their
//     clicks to the window behind is worse than no card), and it means every
//     no-runtime render ends at the toast "Diese Abfrage konnte nicht bedienbar
//     gemacht werden", which is what the first render here actually showed.
//
// So the only state that draws the card is a runtime that ANSWERS - which is
// exactly what this file is, and what --host-script is for.
//
// `?consent=<n>` picks the class, so one file covers the whole polymorphic
// surface and the sweep addresses each by URL. The requests below stand for what
// the BROKER sends; they deliberately mirror the store's own fixture queue, so a
// drift between the two is visible rather than quiet.
(function () {
  var REQUESTS = [
    { id: 1, requester: "org.arlen.files", class: "portal", tier: "standard",
      summary: "open one file you pick", scope: null, reversibility: "reversible" },
    { id: 2, requester: "com.example.notes", class: "capability_grant", tier: "standard",
      summary: "read your notes and their tags", scope: "your notes", reversibility: "reversible" },
    { id: 3, requester: "org.arlen.files", class: "destructive", tier: "standard",
      summary: "move 8 files to the Trash", scope: "~/Downloads", reversibility: "reversible" },
    { id: 4, requester: "org.arlen.files", class: "destructive", tier: "high_stakes",
      summary: "permanently delete 3 files", scope: "~/Documents/old",
      reversibility: "irreversible", total: "1.2 GB", targets: [
        { name: "report-final.pdf", size: "840 MB" },
        { name: "archive-2025.zip", size: "360 MB" },
        { name: "notes.md", size: "4 KB" },
      ] },
    { id: 5, requester: "com.example.mail", class: "external_send", tier: "high_stakes",
      summary: "send an email on your behalf", scope: "alex@example.com",
      reversibility: "irreversible", recipient: "alex@example.com",
      preview: "Subject: Re: Thursday\n\"Sounds good, see you at 3. I'll bring the printouts.\"" },
    { id: 6, requester: "org.arlen.installd", class: "elevated_privilege", tier: "high_stakes",
      summary: "install system software with admin rights", scope: "3 packages",
      reversibility: "reversible_with_cost" },
    { id: 7, requester: "com.example.notes", class: "network_access", tier: "standard",
      summary: "connect to its sync service", scope: "sync.example.com",
      reversibility: "reversible", triggeredExternally: true },
    // NOT a mirror of the store's queue: the single-target destructive case,
    // where the confirm names the file it destroys rather than reading plain.
    // That label is the longest string this surface can produce and the one that
    // used to eat the instruction, so it is worth a shot of its own.
    { id: 8, requester: "org.arlen.files", class: "destructive", tier: "high_stakes",
      summary: "permanently delete a file", scope: "~/Documents/archive",
      reversibility: "irreversible", total: "2.4 GB", targets: [
        { name: "Steuerunterlagen-2024-vollstaendig.tar.zst", size: "2.4 GB" },
      ] },
  ];

  var picked = Number(new URLSearchParams(location.search).get("consent"));
  if (!Number.isInteger(picked) || picked < 0 || picked >= REQUESTS.length) picked = 0;
  var pending = REQUESTS[picked];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      // The broker's answer. The dialog polls this every second and arms once
      // per request id, so returning the same view is the same thing a real
      // broker does while nobody has answered yet.
      if (cmd === "consent_fetch") return Promise.resolve(pending);
      // The two calls the card's own safety gate depends on. Granting the input
      // region is what makes the surface answerable; `consent_ready` is what the
      // window arms on. Refusing either is a state worth its own host file, not
      // this one.
      if (cmd === "set_consent_input_region") return Promise.resolve(null);
      if (cmd === "consent_ready") return Promise.resolve(null);
      if (cmd === "frontend_log") return Promise.resolve(null);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "consent" }, currentWebview: { label: "consent" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
