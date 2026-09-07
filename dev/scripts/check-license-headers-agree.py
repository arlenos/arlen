#!/usr/bin/env python3
"""A file's own SPDX header must not disagree with the licence map, unsaid.

WHY THIS IS NOT WHAT `reuse lint` CHECKS. CI runs `reuse lint`, and it passed
through every case this gate was written for. It has to: a per-file
`SPDX-License-Identifier` header is valid REUSE whatever it says, and REUSE.toml
declares `precedence = "closest"`, which means the file's own header WINS over
the component map above it. So a header that says the wrong thing is not a
malformed file - it is a file that quietly relicenses itself, and the tool whose
job is to find unlabelled files has no opinion about it.

WHAT IT FOUND ON ITS FIRST RUN, which is why it exists. Eight files:
`daemons/kernel-layer/.../tracepoint_layout.rs` carrying AGPL-3.0-only inside the
GPL-2.0-only eBPF crate (AGPL-3.0 is not GPL-2.0-compatible, and the crate's own
Cargo.toml says GPL-2.0-only), and seven first-party app files carrying
GPL-3.0-only where `LICENSING.md` maps `apps/**` to AGPL-3.0-only. Six of those
seven sat in one directory beside two neighbours that said AGPL - the shape of a
header copied from the file open in the next tab, not of a decision.

HOW IT RESOLVES THE MAP. The same two rules REUSE.toml states at its own top: a
later `[[annotations]]` block overrides an earlier one for an overlapping path,
and the file's own header wins over both. This re-implements the glob match
rather than shelling out to `reuse`, because the pre-commit hook runs on a laptop
that does not have it installed.

THE CARVE-OUTS ARE THE POINT, not an exception to it. Lifted code KEEPS its
upstream licence (copy-policy.md), so a disagreement there is correct and has to
be recordable. `KNOWN` below names each one with the reason, so adding a
carve-out is a line somebody writes on purpose and a reader can check, and a
disagreement nobody has explained is what is left over.

WHAT IT DOES NOT DO. It does not judge whether the map itself is right, and it
does not ask for a header: this tree licenses in bulk and most files carry none.
It only compares the two statements when both exist.
"""

import pathlib
import re
import subprocess
import sys

# A root may be given so a control can run this over a fixture instead of the
# tree, the same way the other gates take one.
ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]

# Files whose header is DELIBERATELY not the map's licence, with the reason.
# Every one of these is lifted from a project that is not ours, and the licence
# it arrived under is the one that governs it.
KNOWN = {
    "apps/desktop-shell/src-tauri/src/layer_shell.rs":
        "derived from gtk-layer-shell (GPL-3.0), noted in REUSE.toml's own header",
    "apps/desktop-shell/src-tauri/protocols/arlen-shell-overlay.xml":
        "a Wayland protocol XML, upstream GPL-2.0-or-later",
    "apps/desktop-shell/src-tauri/protocols/arlen-titlebar-v1.xml":
        "a Wayland protocol XML, upstream GPL-3.0-only",
    "apps/desktop-shell/src-tauri/protocols/arlen-window-attach-v1.xml":
        "a Wayland protocol XML, upstream GPL-2.0-or-later",
    "dev/mkosi/mkosi.extra/etc/systemd/system/default.target":
        "systemd's own unit, shipped verbatim under LGPL-2.1-or-later",
}

# An SPDX expression, not "the rest of the line". The first cut took everything
# up to the newline and then read its own source: this file names the tag in its
# docstring and in this very pattern, and the gate reported itself as a file
# licensed `\\s`. Requiring the identifier charset means a mention in prose or
# inside a regex is not a header, and a header still is one.
IDENT = r"[A-Za-z0-9.+()-]+"
HEADER = re.compile(
    rf"SPDX-License-Identifier:[ \t]*({IDENT}(?:[ \t]+(?:OR|AND|WITH)[ \t]+{IDENT})*)"
)


def glob_to_re(pattern: str) -> re.Pattern:
    """A REUSE.toml path glob as a regex: `**` crosses `/`, a single `*` does not."""
    out, i = [], 0
    while i < len(pattern):
        if pattern.startswith("**", i):
            out.append(".*")
            i += 2
        elif pattern[i] == "*":
            out.append("[^/]*")
            i += 1
        else:
            out.append(re.escape(pattern[i]))
            i += 1
    return re.compile("".join(out) + r"\Z")


def annotations(path: pathlib.Path) -> list[tuple[list[re.Pattern], str]]:
    """The map, in file order, because a later block overrides an earlier one."""
    try:
        import tomllib
    except ModuleNotFoundError:  # pragma: no cover - python < 3.11
        print("check-license-headers-agree: needs python 3.11 for tomllib")
        raise SystemExit(1)
    doc = tomllib.loads(path.read_text(encoding="utf-8"))
    blocks = []
    for a in doc.get("annotations", []):
        paths = a["path"] if isinstance(a["path"], list) else [a["path"]]
        lic = a.get("SPDX-License-Identifier")
        if lic:
            blocks.append(([glob_to_re(p) for p in paths], lic))
    return blocks


def mapped(rel: str, blocks) -> str | None:
    """What the map says about one file, last matching block winning."""
    found = None
    for pats, lic in blocks:
        if any(p.match(rel) for p in pats):
            found = lic
    return found


def tracked(root: pathlib.Path) -> list[str]:
    """Every file to compare. Asks git in a repository, walks a fixture tree."""
    try:
        out = subprocess.run(["git", "ls-files"], cwd=root, capture_output=True, text=True, check=True)
        return out.stdout.split()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return sorted(
            str(p.relative_to(root)) for p in root.rglob("*")
            if p.is_file() and ".git" not in p.parts
        )


def own_licence(path: pathlib.Path) -> str | None:
    """The identifier a file states about itself, read from its opening bytes."""
    try:
        head = path.open("rb").read(4096).decode("utf-8", "replace")
    except OSError:
        return None
    m = HEADER.search(head)
    return m.group(1).strip() if m else None


def main() -> int:
    """Refuse a file that relicenses itself away from the map without saying why."""
    toml = ROOT / "REUSE.toml"
    if not toml.is_file():
        print("check-license-headers-agree: no REUSE.toml, which is itself wrong")
        return 1
    blocks = annotations(toml)

    checked, bad = 0, []
    for rel in tracked(ROOT):
        own = own_licence(ROOT / rel)
        if own is None:
            continue
        checked += 1
        want = mapped(rel, blocks)
        if want is None or own == want:
            continue
        if rel in KNOWN:
            continue
        bad.append(
            f"{rel}: the file says {own}, the map in REUSE.toml says {want}.\n"
            f"    Correct the header, or record it in KNOWN with the reason it is lifted."
        )

    for b in bad:
        print(b)
    if bad:
        print(f"\n{len(bad)} file(s) disagreeing with the licence map")
        return 1
    print(f"check-license-headers-agree: {checked} headers agree with the map, {len(KNOWN)} recorded carve-outs")
    return 0


if __name__ == "__main__":
    sys.exit(main())
