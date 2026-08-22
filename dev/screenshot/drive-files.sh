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
// What the window SAYS it did. A row leaving the list is feedback of a kind;
// where the file went, and that the app can put it back, was said nowhere.
// Matched on the SENTENCE, not on the word: the sidebar has a "Trash" place in
// it and `/Trash[^\n]*/` finds that one first, which is what this probe did on
// its first run - it reported "Trash" and I nearly filed the status line as
// missing when it was the reader looking in the wrong place.
out.saidAfterDelete = (document.body.innerText.match(/Moved to Trash[^\n]*/) || [""])[0].trim();
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
say "and the window says where it went, and how to get it back" \
  "$(printf '%s' "$got" | grep -q "Moved to Trash" \
     && printf '%s' "$got" | grep -q "Ctrl+Z" && echo 1 || echo 0)" "$got"

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


# Rename and search each get their own run and their own fixture state, for the
# same reason the delete and undo do: a check after the fact cannot attribute a
# change to the step that should have caused it.
navigate='
const wait = ms => new Promise(r => setTimeout(r, ms));
const out = {};
await wait(3000);
const cellFor = n => [...document.querySelectorAll("*")]
  .filter(e => e.children.length === 0 && (e.textContent||"").trim() === n)[0];
const listing = () => [...document.querySelectorAll(".fm-browse *")]
  .filter(e => e.children.length === 0 && /\.txt$|^sub$/.test((e.textContent||"").trim()))
  .map(e => e.textContent.trim()).sort();
const folder = cellFor("arlen-drive-files");
if (!folder) return JSON.stringify({ step: "navigate", found: false });
(folder.closest("[role=row], li, tr, div") || folder)
  .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
await wait(1800);
out.opened = listing();
'

{ printf '%s' "$navigate"; cat <<'JS'
const a = cellFor("alpha.txt");
if (!a) return JSON.stringify({ ...out, step: "select" });
const row = a.closest("[role=row], li, tr, div") || a;
row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
row.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
row.click();
await wait(600);
// F2 is handled on `.fm`; it opens an inline editor rather than a dialog, so the
// new name is typed into that input and committed with Enter.
(document.querySelector(".fm") || document.body)
  .dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true, cancelable: true }));
await wait(900);
const input = document.querySelector(".fm-browse input");
out.editor = !!input;
if (!input) return JSON.stringify(out);
Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set
  .call(input, "renamed.txt");
input.dispatchEvent(new Event("input", { bubbles: true }));
input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
await wait(2000);
out.afterRename = listing();
return JSON.stringify(out);
JS
} > "$work/.rename.js"

{ printf '%s' "$navigate"; cat <<'JS'
// Ctrl+F is bound at the layout, so this one goes to the window.
window.dispatchEvent(new KeyboardEvent("keydown",
  { key: "f", ctrlKey: true, bubbles: true, cancelable: true }));
await wait(900);
const box = [...document.querySelectorAll("input")].find(i => i.offsetParent !== null);
out.searchBox = !!box;
if (!box) return JSON.stringify(out);
box.focus();
Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set.call(box, "gamma");
box.dispatchEvent(new Event("input", { bubbles: true }));
box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
await wait(3000);
out.hits = [...document.querySelectorAll("*")]
  .filter(e => e.children.length === 0 && /gamma\.txt/.test(e.textContent||""))
  .map(e => e.textContent.trim()).slice(0, 3);
return JSON.stringify(out);
JS
} > "$work/.search.js"

got=$(SHOOT_INJECT="$work/.rename.js" "$here/shoot-app.sh" "$app" "$here/out/files-rename.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "F2 renames the file on disk" \
  "$(printf '%s' "$got" | grep -q '"afterRename":\["beta.txt","renamed.txt","sub"\]' \
     && [ -e "$work/renamed.txt" ] && [ ! -e "$work/alpha.txt" ] && echo 1 || echo 0)" \
  "$got (on disk: $(ls "$work" | tr '\n' ' '))"
mv "$work/renamed.txt" "$work/alpha.txt" 2>/dev/null

# gamma.txt is in `sub/`, so finding it from the parent is the recursive half.
got=$(SHOOT_INJECT="$work/.search.js" "$here/shoot-app.sh" "$app" "$here/out/files-search.png" 2>&1 \
  | sed -n 's/^inject result: //p')
say "search reaches into a subfolder" \
  "$(printf '%s' "$got" | grep -q 'gamma.txt' && echo 1 || echo 0)" "$got"

# A FILE NOTHING OPENS. The window used to do nothing at all here - the host built
# the sentence and `openPath` dropped it in a `catch` - so the case worth holding
# is not that the open fails, it is that the person is told why.
printf 'not really a pdf\n' > "$work/report.pdf"
# Self-contained rather than built on `$common`: that block does not only define
# helpers, it performs the delete, and a probe about opening a file must not run
# a delete first.
cat > "$work/.noopen.js" <<'JS'
const wait = ms => new Promise(r => setTimeout(r, ms));
const out = {};
await wait(3000);
// Unscoped for the folder itself (it is in the HOME listing, not in
// `.fm-browse` yet), scoped afterwards for the same reason `$common` gives.
const cellFor = name => [...document.querySelectorAll("*")]
  .filter(e => e.children.length === 0 && (e.textContent||"").trim() === name)[0];
const folder = cellFor("arlen-drive-files");
if (!folder) return JSON.stringify({ error: "no work folder in home" });
(folder.closest("[role=row], li, tr, div") || folder)
  .dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
await wait(1800);
const row = cellFor("report.pdf");
if (!row) return JSON.stringify({ error: "no report.pdf row" });
row.dispatchEvent(new MouseEvent("dblclick", { bubbles: true, cancelable: true }));
await wait(2500);
out.status = [...document.querySelectorAll(".status-bar span")].map(s => s.textContent.trim());
return JSON.stringify(out);
JS

got=$(SHOOT_INJECT="$work/.noopen.js" "$here/shoot-app.sh" "$app" "$here/out/files-no-handler.png" 2>&1 \
  | sed -n 's/^inject result: //p')
# WHAT it says depends on where it runs, and both answers are honest. Under this
# harness there is no shell, so the launch socket is not there and the window says
# so verbatim ("This did not open: launch socket i/o: Connection refused"). On the
# image the shell answers and it reads "Nothing on this machine is set up to open
# application/pdf files" - photographed on 21 August in `first-run/18`. What the
# case holds is the thing that was missing until then: the refusal REACHES the
# person instead of being caught and dropped.
say "a file that will not open says so instead of doing nothing" \
  "$(printf '%s' "$got" | grep -qE "is set up to open|did not open" && echo 1 || echo 0)" "$got"

[ "$fail" = 0 ] && echo "a folder that opens, a rename and a delete that reach the disk, an undo that restores it, a search that goes deeper than the folder, and a refusal that says why"
exit "$fail"
