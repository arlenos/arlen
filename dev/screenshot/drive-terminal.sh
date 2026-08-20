#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Type into the terminal and read what came back.
#
# WHY THIS EXISTS. The terminal has had two engine-shaped changes - the swap to
# `alacritty_terminal` and the block-mode cutover where xterm.js took over the
# whole surface - and neither had a drive. Its own tests cover the PTY and the
# parser; what none of them can answer is whether a person typing a command sees
# its output, which is the entire app.
#
# It also exists because of how this looked the first time. The screenshot showed
# an apparently empty window and read as a broken terminal; the DOM showed the
# curated zsh prompt exactly where it belongs - a `~` at the top left and the
# clock at the right - with the rest of the screen empty, which is what a fresh
# terminal IS. A drive that asks the DOM cannot make that mistake, and filing a
# bug against a working app is its own kind of damage.
#
# Run: dev/screenshot/drive-terminal.sh [path-to-arlen-terminal]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`: the latter
# leaves the binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-terminal}"
fail=0

[ -x "$app" ] || { echo "no terminal binary at $app"; exit 2; }

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

echo "terminal:"

# What is on screen before anything is typed.
opening=$(SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "$here/out/terminal.png" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
const rows = document.querySelector(".xterm-rows");
return `xterm=${document.querySelectorAll(".xterm").length} prompt=${JSON.stringify((rows?.innerText ?? "").slice(0, 120))}`;
JS
)

say "the terminal surface is there at all" \
  "$(printf '%s' "$opening" | grep -q "xterm=1" && echo 1 || echo 0)" "$opening"

# The curated zsh prompt from TM-R2: the working directory on the left, the
# clock on the right. Its presence is what says a shell actually started - an
# empty grid and a dead PTY look the same in a screenshot.
say "a shell started and drew its prompt" \
  "$(printf '%s' "$opening" | grep -qE 'prompt="[^"]*~' && echo 1 || echo 0)" "$opening"

# AND IT IS THE CURATED ONE. A `~` alone is satisfied by the plain Debian
# `arlen@arlen:~$` too, which is exactly what the terminal falls back to when zsh
# is missing - the state found on the booted image on 20 August, where the
# curated config is installed and nothing can read it. The clock on the right is
# the starship `right_format`, so it is present for the curated prompt and absent
# for the fallback, and with it the block-mode marks that only fire under zsh.
say "and it is the curated prompt, not the fallback shell's" \
  "$(printf '%s' "$opening" | grep -qE 'prompt="[^"]*[0-9]{2}:[0-9]{2}' && echo 1 || echo 0)" "$opening"

# THE case: type a command, press Enter, read the output back off the grid.
# Everything else here is scenery if this does not work.
typed=$(SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "$here/out/terminal-ran.png" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
const ta = document.querySelector(".xterm-helper-textarea");
if (!ta) return "no terminal to type into";
ta.focus();
// THROUGH THE INPUT EVENT, not keydown. xterm.js takes printable characters
// from the helper textarea's `input` event; a synthetic keydown for a letter
// inserts nothing at all. The first cut of this drive used keydown, reported
// `echoed=0`, and would have had me file a bug against a terminal that works -
// which is the exact mistake this script's own header warns about.
const type = (s) => {
  ta.value = s;
  ta.dispatchEvent(new InputEvent("input", { data: s, inputType: "insertText", bubbles: true }));
};
type("echo arlenmarker");
ta.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true }));
const rows = () => document.querySelector(".xterm-rows")?.innerText ?? "";
for (let i = 0; i < 60; i++) {
  if ((rows().match(/arlenmarker/g) ?? []).length >= 2) break;
  await new Promise((r) => setTimeout(r, 100));
}
const text = rows();
const echoed = (text.match(/arlenmarker/g) ?? []).length;
return `echoed=${echoed} blocks=${document.querySelectorAll("[class*=block]").length} tail=${JSON.stringify(text.replace(/\s+/g, " ").slice(-160))}`;
JS
)

# Twice: once as the command the person typed, once as the output the shell
# produced. One occurrence means the keystrokes reached the grid and nothing ran
# them, which is exactly the failure a screenshot cannot tell from success.
say "a typed command runs and its output comes back" \
  "$(printf '%s' "$typed" | grep -qE "echoed=[2-9]" && echo 1 || echo 0)" "$typed"

[ "$fail" = 0 ] && echo "the terminal runs what it is told and shows the answer"
exit "$fail"
