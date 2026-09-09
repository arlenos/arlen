// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Did the `--open` click land? Answers with what the page says about itself.
//
// A probe like the others, and named as one so its control page pairs the way
// every other probe's does (`open-clicked-control.html`). It is not in `$probes`
// - it judges nothing about a route's layout - but it is the same shape: run it
// and read a word out of the answer. What it reports on is the click itself,
// which forty-odd rows of the sweep table depend on and which nothing checked
// until 10 September.
const out = document.getElementById("out");
return [out ? out.textContent : "no #out on the page"];
