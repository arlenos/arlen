#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
#
# Open the document reader on a real PDF and read what it shows.
#
# WHY THIS EXISTS. `apps/pdf/core` is twelve green tests over a document built in
# the test, which proves the parser and proves nothing about the reader: whether
# the outline reaches the screen, whether the depth survives the trip, whether a
# document that opens fine still leaves the window saying nothing is open. Those
# are one `invoke` name apart from working and there is no unit test that can
# tell the difference.
#
# The fixture is generated here rather than committed: a PDF in the tree is a
# file nobody can diff, and the reason each part of it exists - the nested
# outline entry, the word that only appears on page one - belongs beside the
# case that reads it.
#
# Run: dev/screenshot/drive-pdf.sh [path-to-arlen-pdf-app]
#
# Build with `tauri build --no-bundle`, not a plain `cargo build`: the latter
# leaves the binary pointing at devUrl and the run then reports on whatever dev
# server holds that port.
set -uo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/../.." && pwd)"
app="${1:-$root/target/release/arlen-pdf-app}"
fix="$HOME/.cache/arlen-drive-pdf"
fail=0

[ -x "$app" ] || { echo "no reader binary at $app"; exit 2; }

# THE BINARY, NOT THE SOURCE. On 20 August every render case below passed while
# the engine the source names could not load at all: the `target/release` worker
# on disk was still the MuPDF build from four commits earlier, so the drive was
# reporting on an engine the tree had removed - and on an AGPL one, which is the
# reason it was removed. The licence gate reads manifests and cannot see a stale
# binary; a drive that never asks what it is running cannot either.
worker="$root/target/release/arlen-pdf-decode-page"
if [ -x "$worker" ] && strings "$worker" 2>/dev/null | grep -qi mupdf; then
  echo "  FAIL the worker on disk is a MuPDF build, which the tree removed on licence grounds"
  echo "       rebuild it: cargo build --release --manifest-path apps/pdf/decode-page/Cargo.toml"
  exit 1
fi

# Whether this machine can draw a page at all. `pdfium-render` binds to a
# `libpdfium` at RUNTIME and no distribution in play ships one, so the render
# cases below are skipped rather than failed where there is no engine - said out
# loud, in the same words the crate's own tests use, because a silent skip is how
# a suite comes to report success for something that never ran.
engine=1
if ! "$worker" 1 1.0 < "$fix/sample.pdf" > /dev/null 2>&1; then
  engine=0
fi
rm -rf "$fix"
mkdir -p "$fix"

# Three pages, a two-level outline and one word that appears on exactly one page.
# Written as a literal PDF rather than through a library so the file the reader
# opens is the file this script describes.
python3 - "$fix/sample.pdf" <<'PY'
import sys

PAGES = [
    b"BT /F1 12 Tf 72 720 Td (Chapter one begins here with a needle in it) Tj ET",
    b"BT /F1 12 Tf 72 720 Td (The second page continues the argument) Tj ET",
    b"BT /F1 12 Tf 72 720 Td (Method and measurements) Tj ET",
]

def obj(n, body):
    return f"{n} 0 obj\n{body}\nendobj\n"

cat, pages_id, font = 1, 2, 3
n = 4
page_ids, content_ids = [], []
for _ in PAGES:
    page_ids.append(n); n += 1
    content_ids.append(n); n += 1
outlines = n; n += 1
first, child, last = n, n + 1, n + 2

body = obj(cat, f"<< /Type /Catalog /Pages {pages_id} 0 R /Outlines {outlines} 0 R >>")
kids = " ".join(f"{p} 0 R" for p in page_ids)
body += obj(pages_id, f"<< /Type /Pages /Kids [{kids}] /Count {len(page_ids)} >>")
body += obj(font, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")
for i, (pid, cid) in enumerate(zip(page_ids, content_ids)):
    body += obj(pid, f"<< /Type /Page /Parent {pages_id} 0 R /Contents {cid} 0 R "
                     f"/Resources << /Font << /F1 {font} 0 R >> >> /MediaBox [0 0 595 842] >>")
    s = PAGES[i].decode()
    body += obj(cid, f"<< /Length {len(s)} >>\nstream\n{s}\nendstream")
body += obj(outlines, f"<< /Type /Outlines /First {first} 0 R /Last {last} 0 R /Count 3 >>")
body += obj(first, f"<< /Title (Chapter one) /Parent {outlines} 0 R /First {child} 0 R "
                   f"/Last {child} 0 R /Next {last} 0 R /Dest [{page_ids[0]} 0 R /XYZ null null null] >>")
body += obj(child, f"<< /Title (Background) /Parent {first} 0 R "
                   f"/Dest [{page_ids[1]} 0 R /XYZ null null null] >>")
body += obj(last, f"<< /Title (Method) /Parent {outlines} 0 R /Prev {first} 0 R "
                  f"/Dest [{page_ids[2]} 0 R /XYZ null null null] >>")

head = "%PDF-1.5\n"
offsets, pos = [], len(head)
for chunk in body.split("endobj\n")[:-1]:
    offsets.append(pos)
    pos += len(chunk) + len("endobj\n")
xref = f"xref\n0 {len(offsets)+1}\n0000000000 65535 f \n"
xref += "".join(f"{o:010d} 00000 n \n" for o in offsets)
doc = head + body + xref
doc += f"trailer\n<< /Size {len(offsets)+1} /Root {cat} 0 R >>\nstartxref\n{pos}\n%%EOF\n"
open(sys.argv[1], "wb").write(doc.encode("latin-1"))
PY

say() {  # say <name> <ok> <detail>
  if [ "$2" = 1 ]; then echo "  ok   $1"; else echo "  FAIL $1"; echo "       $3"; fail=1; fi
}

echo "pdf reader:"

# Opened ON the file, the way the file manager opens it. `SHOOT_APP_ARGS` is what
# reaches the binary's argv; the first run of this used a knob that does not
# exist and photographed the reader's empty state instead, which looked exactly
# like a reader that cannot open a document.
shot="$here/out/pdf-reader.png"
got=$(SHOOT_APP_ARGS="$fix/sample.pdf" "$here/shoot-app.sh" "$app" "$shot" 2>&1 \
  | sed -n 's/^inject result: //p')
dom=$(SHOOT_APP_ARGS="$fix/sample.pdf" SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
return document.body.innerText.replace(/\s+/g, " ").slice(0, 600);
JS
)

say "a document opened from the file manager is the one shown" \
  "$(printf '%s' "$dom" | grep -qE "of 3|von 3" && echo 1 || echo 0)" "$dom"

# The whole point of reading the outline rather than counting pages: the author's
# own headings, in their own order.
say "the author's headings reach the screen in order" \
  "$(printf '%s' "$dom" | grep -q "Chapter one.*Background.*Method" && echo 1 || echo 0)" "$dom"

# THE case, and the one that would have caught the two defects this drive found
# the hard way. It reads the CANVAS, not the DOM: a page that renders as clean
# white paper is indistinguishable from a page that drew nothing, and both of
# those happened here - once because the pixmap was allocated with alpha (a
# fully transparent raster), once because MuPDF was built with no fonts, so a
# document naming Helvetica got a page with no glyphs and no error. Every other
# case in this file passed through both.
ink=$(SHOOT_APP_ARGS="$fix/sample.pdf" SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
const c = document.querySelector("canvas");
if (!c || !c.width) return "no canvas";
const d = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
let dark = 0, opaque = 0;
for (let i = 0; i < d.length; i += 4) {
  if (d[i + 3] === 255) opaque++;
  if (d[i] < 128 && d[i + 3] > 0) dark++;
}
return `size=${c.width}x${c.height} opaque=${opaque} dark=${dark}`;
JS
)

if [ "$engine" = 0 ]; then
  echo "  --   drawing a page: no libpdfium on this machine, so nothing was rendered"
  # And THAT case is the one every machine is in today, so what the reader does
  # instead is the case worth holding: the page's words, said to be the text and
  # not the page.
  words=$(SHOOT_APP_ARGS="$fix/sample.pdf" SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "$here/out/pdf-reader.png" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
await new Promise((r) => setTimeout(r, 2500));
return (document.body.innerText || "").replace(/\s+/g, " ").trim().slice(0, 400);
JS
)
  say "a page it cannot draw still shows the words" \
    "$(printf '%s' "$words" | grep -q "without its layout" \
       && printf '%s' "$words" | grep -q "Chapter one begins here" && echo 1 || echo 0)" "$words"
else
say "the page is drawn as opaque paper rather than a transparent sheet" \
  "$(printf '%s' "$ink" | grep -qE "opaque=[1-9]" && echo 1 || echo 0)" "$ink"

say "and the document's own text is on it" \
  "$(printf '%s' "$ink" | grep -qE "dark=[1-9]" && echo 1 || echo 0)" "$ink"
fi

# Page navigation, the third thing `quickview-plan.md` names for this reader and
# the last one to arrive. Pressed as a key on the window rather than clicked,
# because keyboard-first is the convention and a reader who has to reach for the
# mouse to turn a page is not reading.
nav=$(SHOOT_APP_ARGS="$fix/sample.pdf" SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
const at = () => document.querySelector(".page-of")?.innerText ?? "?";
const press = (key, shiftKey = false) =>
  window.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey, bubbles: true, cancelable: true }));
const start = at();
press("ArrowRight");
const forward = at();
press("ArrowLeft");
const back = at();
press("End");
const end = at();
// Past the last page: the end of a document is the end of it, and wrapping to
// page one would read as the document having restarted.
press("ArrowRight");
const clamped = at();
return `start=${start} forward=${forward} back=${back} end=${end} clamped=${clamped}`;
JS
)

say "a key turns the page, and turns it back" \
  "$(printf '%s' "$nav" | grep -q "start=Page 1 of 3 forward=Page 2 of 3 back=Page 1 of 3" && echo 1 || echo 0)" "$nav"

say "the last page is the last page, not a wrap to the first" \
  "$(printf '%s' "$nav" | grep -q "end=Page 3 of 3 clamped=Page 3 of 3" && echo 1 || echo 0)" "$nav"

# Text selection, the fourth thing the plan names. The canvas is pixels and
# carries no text a browser can reach, so the page's own lines are laid over it
# as transparent text - and the case that matters is not "a layer exists" but
# whether it sits ON the words: a layer positioned beside them selects nothing a
# reader pointed at and looks identical from the DOM.
sel=$(SHOOT_APP_ARGS="$fix/sample.pdf" SHOOT_INJECT=/dev/stdin "$here/shoot-app.sh" "$app" "" 2>&1 <<'JS' \
  | sed -n 's/^inject result: //p'
const span = document.querySelector(".text-layer span");
const canvas = document.querySelector(".page-canvas");
if (!span || !canvas) return "no text layer";
const s = span.getBoundingClientRect();
const c = canvas.getBoundingClientRect();
const inside = s.left >= c.left - 1 && s.right <= c.right + 1 &&
               s.top >= c.top - 1 && s.bottom <= c.bottom + 1;
// Selected the way a reader does, then read back what the document says was
// selected - the browser's own answer, not ours.
const range = document.createRange();
range.selectNodeContents(span);
const sel = window.getSelection();
sel.removeAllRanges();
sel.addRange(range);
return `text=${JSON.stringify(span.textContent)} inside=${inside} area=${Math.round(s.width)}x${Math.round(s.height)} selected=${JSON.stringify(sel.toString())}`;
JS
)

if [ "$engine" = 0 ]; then
  echo "  --   laying the words over the page: same reason, the boxes are in the raster's own pixel space"
else
say "the page's words are laid over the page, not beside it" \
  "$(printf '%s' "$sel" | grep -q "inside=true" && echo 1 || echo 0)" "$sel"

say "and selecting them gives back what the document says" \
  "$(printf '%s' "$sel" | grep -q 'selected="Chapter one begins here with a needle in it"' && echo 1 || echo 0)" "$sel"
fi

# SEARCH. The word `needle` was planted on page one when this fixture was
# written and nothing had ever looked for it, so the search - the one feature
# that reads the whole document rather than the page in front of you - shipped
# undriven. It matters more than it looks: a search that quietly finds nothing
# is indistinguishable from a document that does not contain the word, and a
# reader believes the second.
cat > "$fix/p-search.js" <<'JS'
const box = document.querySelector('input[type=search]');
if (!box) return "no search box";
const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value").set;
set.call(box, "needle");
box.dispatchEvent(new Event("input", { bubbles: true }));
for (let i = 0; i < 60; i++) {
  await new Promise((r) => setTimeout(r, 100));
  if (document.querySelectorAll(".pdf-hits li").length) break;
}
const hits = [...document.querySelectorAll(".pdf-hits li")].map((li) => li.innerText.replace(/\s+/g, " ").trim());
// Jump to the hit and report which page the reader is on afterwards, since a
// result list nobody can act on is a list, not a search.
const before = (document.querySelector(".pdf-page-indicator, [class*=indicator]")?.innerText ?? "").trim();
document.querySelector(".pdf-hits .hit")?.click();
await new Promise((r) => setTimeout(r, 600));
const dom = document.body.innerText.replace(/\s+/g, " ").trim();
// And a word that is not in the document, to prove the first answer was about
// the document rather than about the search always saying yes.
set.call(box, "zzzznotpresent");
box.dispatchEvent(new Event("input", { bubbles: true }));
for (let i = 0; i < 40; i++) {
  await new Promise((r) => setTimeout(r, 100));
  if (!document.querySelectorAll(".pdf-hits li").length) break;
}
const absent = document.body.innerText.replace(/\s+/g, " ").trim();
return `hits=${JSON.stringify(hits.join(" | ").slice(0, 200))} after=${JSON.stringify(dom.slice(0, 200))} `
  + `absent=${JSON.stringify(absent.slice(0, 160))} before=${JSON.stringify(before)}`;
JS
found=$(SHOOT_APP_ARGS="$fix/sample.pdf" SHOOT_INJECT="$fix/p-search.js" \
  "$here/shoot-app.sh" "$app" "$here/out/pdf-search.png" 2>&1 \
  | sed -n 's/^inject result: //p')

# One hit, and on the page the word is actually on. A search that returned every
# page would also be "not empty".
say "a word in the document is found on the page that has it" \
  "$(printf '%s' "$found" | grep -qE 'hits="[^"]*needle' && echo 1 || echo 0)" "$found"

say "and only on that page" \
  "$(case "$found" in ""|REFUSED:*) echo 0;; *) printf '%s' "$found" | grep -q "|" && echo 0 || echo 1;; esac)" "$found"

# A word nobody wrote must be answered, not ignored. This is the sentence that
# stops an empty result reading as a broken search.
say "a word that is not there is answered rather than left blank" \
  "$(printf '%s' "$found" | grep -qE 'absent="[^"]*(No page contains that)' && echo 1 || echo 0)" "$found"

# Nothing here is a fixture string: a document with no contents page and a
# document that failed to open are different, and the second must not be
# reported as the first.
say "opening a document did not leave it saying nothing is open" \
  "$(case "$dom" in ""|REFUSED:*) echo 0;; *) printf '%s' "$dom" | grep -q "No document is open" && echo 0 || echo 1;; esac)" "$dom"

# A DOCUMENT WITH A PASSWORD ON IT. Bank statements and payslips arrive like
# this, and until 22 August the window said "Could not open this document: this
# file could not be read as a PDF" - which sends a person looking for a corrupt
# download. The fixture is the core's own, written by `qpdf --encrypt`.
locked="$root/apps/pdf/core/testdata/user-locked.pdf"
if [ -f "$locked" ]; then
  cat > "$fix/p-locked.js" <<'JS'
await new Promise(r => setTimeout(r, 2000));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 300);
JS
  lk=$(SHOOT_APP_ARGS="$locked" SHOOT_INJECT="$fix/p-locked.js" \
    "$here/shoot-app.sh" "$app" "$here/out/pdf-locked.png" 2>&1 | sed -n 's/^inject result: //p')
  say "a document with a password says so, and does not call itself damaged" \
    "$(printf '%s' "$lk" | grep -q "locked with a password" \
       && ! printf '%s' "$lk" | grep -q "could not be read as a PDF" && echo 1 || echo 0)" "$lk"
else
  say "the locked fixture is where the core keeps it" 0 "missing $locked"
fi

# LAUNCHED WITH NOTHING, the state the launcher gives a person. This reader cannot
# open a document itself - it takes a path from `%f` or argv - and its sentence is
# the one the mail and viewer windows were brought up to, so it is worth holding in
# place rather than trusting that it stays.
cat > "$fix/p-bare.js" <<'JS'
await new Promise(r => setTimeout(r, 2000));
return document.body.innerText.replace(/\s+/g, " ").trim().slice(0, 200);
JS

bare=$(SHOOT_INJECT="$fix/p-bare.js" \
  "$here/shoot-app.sh" "$app" "$here/out/pdf-no-file.png" 2>&1 | sed -n 's/^inject result: //p')

say "launched with no document, it says where one comes from" \
  "$(printf '%s' "$bare" | grep -qE "Files|Dateien" && echo 1 || echo 0)" "$bare"

[ "$fail" = 0 ] && echo "the reader opens a real document, says what is in it, and an empty window says where to get one"
exit "$fail"
