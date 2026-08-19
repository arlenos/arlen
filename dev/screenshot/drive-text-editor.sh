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

[ "$fail" = 0 ] && echo "a real buffer over a real file, a find panel, and a save that lands on disk"
exit "$fail"
