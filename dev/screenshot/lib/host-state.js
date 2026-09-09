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
return document.body.innerText.replace(/\s+/g, " ");
