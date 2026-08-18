#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the file manager: navigate into a folder, trash a file, take it back
# with Ctrl+Z - and check the filesystem after each step, not the listing.
#
# WHY THE DISK AND NOT THE LIST. `files_op` records an undo entry per operation
# and a permanent delete records none, so "undo returned true" and "the row came
# back" are both weaker claims than the one that matters: the file is on disk
# again. The listing can be right while the filesystem is not.
#
# THE CLICK PATH, NOT THE COMMANDS. `withGlobalTauri` is off, so an injected
# probe has no `window.__TAURI__` and cannot call the 41 commands directly. That
# is the right setting and it makes this the better test: everything here goes
# through the frontend, the IPC and the backend the way a person's hands would.
#
# WHERE THE KEYS GO, since it differs per app and getting it wrong reads as a
# missing feature: Delete is handled on the `.fm` container, Ctrl+Z at the
# layout. The viewer's Delete is on `svelte:window`; the editor's Ctrl+S is
# inside CodeMirror.
#
# Run: dev/screenshot/drive-files.sh [path-to-arlen-files]
#
# Build with `tauri build --no-bundle`; a plain `cargo build --release` leaves the
# binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-files}"
# Directly under $HOME, and without a leading dot: the app opens at Home and
# hides dotfiles, so a fixture in `~/.cache` cannot be reached by clicking.
work="$HOME/arlen-drive-files"
fail=0

[ -x "$app" ] || { echo "no files binary at $app"; exit 2; }
rm -rf "$work"; mkdir -p "$work/sub" "$here/out"
printf 'one\n' > "$work/alpha.txt"
printf 'two\n' > "$work/beta.txt"
printf 'deep\n' > "$work/sub/gamma.txt"

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

# TWO runs, not one. A single probe that deletes and then undoes leaves the file
# present at the end, so a shell check afterwards cannot tell "the delete never
# happened" from "the undo put it back" - the first version of this script called
# a working delete a failure for exactly that reason. Each run ends where its own
# disk check can settle it.
common='
const wait = ms => new Promise(r => setTimeout(r, ms));
const out = {};
await wait(3000);
const cellFor = name => [...document.querySelectorAll("*")]
  .filter(e => e.children.length === 0 && (e.textContent||"").trim() === name)[0];
// Scoped to `.fm-browse`, because the sidebar places list matches a bare
// every-leaf scan and reads as if the folder never opened.
const listing = () => [...document.querySelectorAll(".fm-browse *")]
  .filter(e => e.children.length === 0 && /\.txt$|^sub$/.test((e.textContent||"").trim()))
  .map(e => e.textContent.trim()).sort();
const folder = cellFor("arlen-drive-files");
if (!folder) return JSON.stringify({ step: "navigate", found: false });
(folder.closest("[role=row], li, tr, div") || folder)
  .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
await wait(1800);
out.opened = listing();
const beta = cellFor("beta.txt");
if (!beta) return JSON.stringify({ ...out, step: "select", found: false });
const row = beta.closest("[role=row], li, tr, div") || beta;
row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
row.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
row.click();
await wait(600);
(document.querySelector(".fm") || document.body)
  .dispatchEvent(new KeyboardEvent("keydown", { key: "Delete", bubbles: true, cancelable: true }));
await wait(2000);
out.afterDelete = listing();
'

{ printf '%s' "$common"; echo 'return JSON.stringify(out);'; } > "$work/.delete.js"
{ printf '%s' "$common"; cat <<'JS'
window.dispatchEvent(new KeyboardEvent("keydown",
  { key: "z", ctrlKey: true, bubbles: true, cancelable: true }));
await wait(2000);
out.afterUndo = listing();
return JSON.stringify(out);
JS
} > "$work/.undo.js"

echo "file manager:"

got=$(SHOOT_INJECT="$work/.delete.js" "$here/shoot-app.sh" "$app" "$here/out/files-delete.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "opens a folder and lists what is in it" \
  "$(printf '%s' "$got" | grep -q '"opened":\["alpha.txt","beta.txt","sub"\]' && echo 1 || echo 0)" "$got"
say "Delete takes the file off the disk, not just out of the list" \
  "$(printf '%s' "$got" | grep -q '"afterDelete":\["alpha.txt","sub"\]' \
     && [ ! -e "$work/beta.txt" ] && echo 1 || echo 0)" \
  "$got (beta.txt on disk: $([ -e "$work/beta.txt" ] && echo PRESENT || echo gone))"

printf 'two\n' > "$work/beta.txt"
got=$(SHOOT_INJECT="$work/.undo.js" "$here/shoot-app.sh" "$app" "$here/out/files-undo.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "Ctrl+Z puts it back on disk with its contents" \
  "$(printf '%s' "$got" | grep -q '"afterUndo":\["alpha.txt","beta.txt","sub"\]' \
     && [ -e "$work/beta.txt" ] && [ "$(cat "$work/beta.txt")" = two ] && echo 1 || echo 0)" \
  "$got (beta.txt on disk: $([ -e "$work/beta.txt" ] && echo present || echo GONE))"

[ "$fail" = 0 ] && echo "a folder that opens, a delete that reaches the disk, and an undo that restores it with its contents"
exit "$fail"
