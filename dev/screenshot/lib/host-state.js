// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// What the page says, as one line, so a host row can be checked against the
// `// EXPECT:` its fixture declares.
//
// `innerText` rather than `textContent`, and it is load-bearing: `innerText`
// omits what is not rendered. The jobs fixture was once clicking a Cancel button
// inside a CLOSED popover - the refusal existed at `visibility: hidden`, and
// `textContent` would have found the sentence and passed a fixture whose state
// nobody could see. `probe-host.sh` reads the page the same way for the same
// reason; this file exists so the sweep can do it without shelling out to that.
// AS AN ARRAY, like every probe, so a reader can pick the answer out of the
// run's output by shape. Returning a bare string meant the only way to find it
// was "the last line", and one run in ten puts a MESA driver warning there -
// which then reads as a page that says nothing about itself. `probe-host.sh`
// carries a comment about that exact warning from an earlier round.
return [document.body.innerText.replace(/\s+/g, " ")];
