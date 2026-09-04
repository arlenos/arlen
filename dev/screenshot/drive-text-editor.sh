#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Drive the text editor through what `text-editor-app.md` names: CodeMirror 6,
# real file open and save, syntax modes, find.
#
# THE SAVE CASE writes to a scratch file this script makes, and reads the file
# back off disk afterwards. That is the only way to tell a save from a status
# line that says "Saved": the editor computed `dirty`, `savedAt` and `saveError`
# for a while before any of them reached the markup, so a failed write was
# invisible - the defect its own source comments about.
#
# WHERE THE KEYS GO. Ctrl+S is a CodeMirror keymap, so it must be aimed at
# `.cm-content`; the viewer's Delete is a `svelte:window` handler and must be
# aimed at the window. Getting that backwards produces a probe that presses
# nothing and reads exactly like a feature that is missing.
#
# Run: dev/screenshot/drive-text-editor.sh [path-to-arlen-text-editor]
#
# Build with `tauri build --no-bundle`; a plain `cargo build --release` leaves the
# binary pointing at devUrl.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-text-editor}"
work="$HOME/.cache/arlen-drive-editor"
fail=0

[ -x "$app" ] || { echo "no text-editor binary at $app"; exit 2; }
mkdir -p "$work" "$here/out"
printf 'fn main() {\n    let greeting = "hallo";\n    println!("{greeting}");\n}\n' > "$work/sample.rs"
cp "$work/sample.rs" "$work/sample.rs.before"

say() {
  local name="$1" ok="$2" got="$3"
  if [ "$ok" = 1 ]; then echo "  ok   $name"; else echo "  FAIL $name"; echo "       $got"; fail=1; fi
}

drive_bare() {  # drive_bare <probe-js> <out-png> - launched with NO file
  # The state a person gets from the launcher, which nothing here used to open.
  # Every drive in this directory hands the app a file, so the empty state went
  # unphotographed - and three apps drifted into three different sentences for it.
  SHOOT_INJECT="$1" "$here/shoot-app.sh" "$app" "$here/out/$2" 2>&1 \
    | sed -n 's/^inject result: //p'
}

drive() {  # drive <probe-js> <out-png>
  local out
  out="$(SHOOT_APP_ARGS="$work/sample.rs" SHOOT_INJECT="$1" \
    "$here/shoot-app.sh" "$app" "$here/out/$2" 2>&1)"
  # A debug binary loads its devUrl, so with nothing serving that port every probe
  # runs against a connection-refused page and returns nothing. `shoot-app.sh` says
  # so plainly; this used to keep only the `inject result:` lines, which threw that
  # sentence away and reported three FEATURES as broken. The cause was one missing
  # server, and looking for it in the editor is a wasted hour.
  # `drive` runs inside `$( )`, so an `exit` here would only leave the subshell -
  # the first attempt printed this three times and still reported three broken
  # features. The marker file carries the verdict back out.
  if printf '%s' "$out" | grep -q "SHOT IS AN ERROR PAGE"; then
    : > "$work/no-frontend"
  fi
  printf '%s' "$(printf '%s' "$out" | sed -n 's/^inject result: //p')"
}

echo "text editor:"

cat > "$work/p-open.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const cm = document.querySelector(".cm-content");
return JSON.stringify({
  buffer: !!cm,
  text: cm ? cm.textContent.replace(/\s+/g, " ").trim().slice(0, 60) : null,
  // A grammar that loaded paints spans inside the lines; plain text has none.
  tokens: document.querySelectorAll(".cm-line span").length,
});
JS
rm -f "$work/no-frontend"
got=$(drive "$work/p-open.js" editor-open.png)
if [ -e "$work/no-frontend" ]; then
    echo "  nothing was tested: no frontend is being served, so every probe ran"
    echo "  against a connection-refused page. A debug binary loads its devUrl."
    echo "  Build with \`tauri build --no-bundle\`, or serve that port:"
    echo "    (cd $root/apps/text-editor && npx vite build && npx vite preview --port 1431)"
    exit 2
fi
say "opens a real file into a CodeMirror buffer, with its grammar loaded" \
  "$(printf '%s' "$got" | grep -q '"buffer":true' \
     && printf '%s' "$got" | grep -q 'let greeting' \
     && printf '%s' "$got" | grep -qE '"tokens":[1-9]' && echo 1 || echo 0)" "$got"

cat > "$work/p-find.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const cm = document.querySelector(".cm-content");
if (!cm) return "no buffer";
cm.focus();
cm.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true }));
await new Promise(r => setTimeout(r, 800));
const panel = document.querySelector(".cm-search, .cm-panel");
return JSON.stringify({ panel: !!panel,
  text: panel ? (panel.textContent||"").replace(/\s+/g," ").trim().slice(0,60) : null });
JS
got=$(drive "$work/p-find.js" editor-find.png)
say "find opens its panel on the editor's own keymap" \
  "$(printf '%s' "$got" | grep -q '"panel":true' && echo 1 || echo 0)" "$got"

cat > "$work/p-save.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const cm = document.querySelector(".cm-content");
if (!cm) return "no buffer";
cm.focus();
document.execCommand("insertText", false, "// driven\n");
await new Promise(r => setTimeout(r, 600));
cm.dispatchEvent(new KeyboardEvent("keydown",
  { key: "s", ctrlKey: true, bubbles: true, cancelable: true }));
await new Promise(r => setTimeout(r, 1500));
const state = document.querySelector(".savestate");
return JSON.stringify({ state: state && state.textContent.trim() });
JS
got=$(drive "$work/p-save.js" editor-save.png)
changed=$(cmp -s "$work/sample.rs.before" "$work/sample.rs" && echo no || echo yes)
say "typing and saving reaches the file on disk, not just the status line" \
  "$([ "$changed" = yes ] && head -1 "$work/sample.rs" | grep -q "// driven" && echo 1 || echo 0)" \
  "$got (file changed: $changed, first line: $(head -1 "$work/sample.rs"))"

# Printing. The same portal call the viewer makes, from the plugin both share,
# so a document reaches a printer and not only a picture. The FILE is printed,
# not the buffer, and this runs against the real portal on this machine - the
# app is killed at the end of the shot, which drops the request.
cat > "$work/p-print.js" <<'JS'
const b = document.querySelector('[aria-label="Print"], [aria-label="Drucken"]');
if (!b) return "no print control in the toolbar";
b.click();
await new Promise(r => setTimeout(r, 900));
return JSON.stringify({ state: (document.querySelector('[role="status"]')||{}).textContent,
  body: (document.body.innerText||"").replace(/\s+/g," ").trim().slice(0,120) });
JS
got=$(drive "$work/p-print.js" editor-print.png)
say "the print control hands the open file to the portal and says the request is pending" \
  "$(printf '%s' "$got" | grep -qiE "print service|Druckdienst" && echo 1 || echo 0)" "$got"

# The lost update. Open a file, let something else write it, then save: the
# editor must refuse rather than destroy the other change. Driven because the
# unit tests prove the host refuses and this proves the SURFACE asks.
cat > "$work/p-clobber.js" <<'JS'
await new Promise(r => setTimeout(r, 2500));
const cm = document.querySelector(".cm-content");
if (!cm) return "no buffer";
cm.focus();
document.execCommand("insertText", false, "// mine\n");
// The other writer lands during this wait, after the file was opened and
// before the save - which is exactly the window a lost update lives in.
// The other writer lands at twelve seconds, after this window opened the file
// and before it saves - which is exactly where a lost update lives. The gaps are
// wide because the ordering is by wall clock and the app's launch time varies by
// seconds between a warm and a cold run: at four seconds this case passed and
// then failed on the next run, having opened the file AFTER the other writer, so
// the save was legitimate and the guard had nothing to refuse. The probe runs
// inside the webview and cannot touch the filesystem to synchronise properly.
await new Promise(r => setTimeout(r, 11500));
cm.dispatchEvent(new KeyboardEvent("keydown",
  { key: "s", ctrlKey: true, bubbles: true, cancelable: true }));
await new Promise(r => setTimeout(r, 1200));
return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 300);
JS
( sleep 12; printf '// somebody else\n' > "$work/sample.rs" ) &
got=$(drive "$work/p-clobber.js" editor-clobber.png)
wait
say "a save over a file that changed on disk is refused, and the page says so" \
  "$(printf '%s' "$got" | grep -qiE "changed on disk|auf der Festplatte geändert" && echo 1 || echo 0)" "$got"
# Exactly the other writer's file, not merely containing its line: a save that
# went through would leave the buffer's text here too, and `grep` would pass.
say "and the other change is still on disk, untouched" \
  "$([ "$(cat "$work/sample.rs")" = "// somebody else" ] && echo 1 || echo 0)" \
  "$(cat "$work/sample.rs")"

cat > "$work/p-bare.js" <<'JS'
await new Promise(r => setTimeout(r, 2000));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 220);
JS

bare=$(drive_bare "$work/p-bare.js" editor-no-file.png)

# WITH NO FILE, THE WINDOW SHOWS A DOCUMENT THAT IS NOT ON THE MACHINE. The two
# demos describe the editor itself and cannot be saved, which is careful - but the
# picker shows a filename and the lens beside it answers real queries about that
# name and comes back empty, which reads as a file this machine has and the graph
# knows nothing about. It is not here at all, and the window has to say so.
say "launched with no file, it says the document is an example" \
  "$(printf '%s' "$bare" | grep -q "Example document" && echo 1 || echo 0)" "$bare"

# And it must not offer to save invented text under an invented name.
say "and offers no save over it" \
  "$(case "$bare" in ""|REFUSED:*) echo 0;; *) printf '%s' "$bare" | grep -qE 'Save|Speichern' && echo 0 || echo 1;; esac)" "$bare"

[ "$fail" = 0 ] && echo "a real buffer over a real file, a find panel, a save that lands on disk, and a sample that says it is one"
exit "$fail"
