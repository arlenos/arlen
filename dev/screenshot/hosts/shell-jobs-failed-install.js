// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// EXPECT: package not found
//
// The Activity zone showing a job that ENDED BADLY and counted nothing - the
// shape an install failure has, and until 7 September a shape nothing could
// reach: the store dropped a finished row the instant the producer removed it,
// so the message was on screen for no frames at all.
//
// TWO THINGS THIS PHOTOGRAPHS, and neither has another home:
//  - the failure sentence, which is the whole reason a receipt lingers;
//  - the absence of a progress bar. A job with no counted total reports no
//    metric, and drawing a determinate bar from its fraction of zero said the
//    work was stuck. Only the picture shows that the bar is gone.
//
// NO GESTURE. Unlike its sibling `shell-jobs-refuse-cancel`, this state is the
// first render: the zone lists what the store holds, and the store holds what
// this runtime answers. There is nothing to press.
//
// RUN IT AGAINST `/_jobstest`, not the shell's main route - the same reason its
// sibling carries: the popovers do not open for a scripted click, which is why
// that route exists.
(function () {
  var JOBS = [
    {
      id: "install-1",
      // The producer writes a job's title, so it is the daemon's prose and not
      // ours (`jobs.ts` records that decision and the open question under it).
      title: "Installing notes-1.2",
      appId: "arlen-installd",
      appLabel: "arlen-installd",
      // Zero, and MEANINGLESS without the flag below - which is the defect this
      // fixture stands over. An install measures nothing.
      fraction: 0,
      determinate: false,
      state: "error_fatal",
      // Empty rather than a zero row: "0 of 0 items" is a sentence about a total
      // nobody counted.
      metrics: [],
      // A transactional install has no cancel path, so the row offers no button.
      killable: false,
      suspendable: false,
      error: "package not found: notes-1.2.lunpkg",
      items: [],
      startedAt: 1_756_000_000_000_000,
    },
  ];

  window.__TAURI_INTERNALS__ = {
    invoke: function (cmd) {
      if (cmd === "list_jobs") return Promise.resolve(JOBS);
      return Promise.reject("stub-host: no backend behind this window (" + cmd + ")");
    },
    transformCallback: function (cb) { return cb; },
    metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main" } },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: function () {} };
})();
