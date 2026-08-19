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
  "$(printf '%s' "$dom" | grep -q "3 pages" && echo 1 || echo 0)" "$dom"

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

say "the page is drawn as opaque paper rather than a transparent sheet" \
  "$(printf '%s' "$ink" | grep -qE "opaque=[1-9]" && echo 1 || echo 0)" "$ink"

say "and the document's own text is on it" \
  "$(printf '%s' "$ink" | grep -qE "dark=[1-9]" && echo 1 || echo 0)" "$ink"

# Nothing here is a fixture string: a document with no contents page and a
# document that failed to open are different, and the second must not be
# reported as the first.
say "opening a document did not leave it saying nothing is open" \
  "$(printf '%s' "$dom" | grep -q "No document is open" && echo 0 || echo 1)" "$dom"

[ "$fail" = 0 ] && echo "the reader opens a real document and says what is in it"
exit "$fail"
