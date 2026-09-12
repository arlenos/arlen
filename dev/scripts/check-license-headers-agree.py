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

THE SECOND RULE: A FIXTURE LICENCE MUST BE MARKED AS ONE. `reuse lint` reads
every file in the tree and cannot tell a test's SAMPLE expression from a real
declaration, so a test that plants a sample identifier line to feed a gate has
that sample read as the test file's own licence. On 12 September that turned the
`license` job red: a fixture SVG header inside a JS string was extracted as
`CC0-1.0 -->`, trailing markup and all, which is not a valid expression. The
documented answer is `REUSE-IgnoreStart` / `REUSE-IgnoreEnd` around the samples.

So an expression BEYOND a file's own header needs those markers. It catches the
silent half too: a fixture whose sample happens to be a valid id does not fail
`reuse`, it is simply read as that file's licence and nobody notices. This rule is
here rather than in a gate of its own because it is the same subject - what a file
says about its own licence - and because the next test that plants a header should
inherit the check without anybody remembering to wire one up.
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
# The tag is ASSEMBLED rather than written out, and that is not decoration.
# `reuse lint` scans every file for this exact string and reads what follows as
# the file's own licence: with the pattern spelled out, REUSE parsed
# `[ \t]*({IDENT}...` as an SPDX expression and the licence job went red on the
# commit that added this gate. Keeping the two halves apart puts the pattern out
# of REUSE's reach without teaching REUSE to skip a file - a suppression that
# hides this one today hides a real one later. The control pins it.
TAG = "SPDX-License" "-Identifier:"
HEADER = re.compile(
    rf"{re.escape(TAG)}[ \t]*({IDENT}(?:[ \t]+(?:OR|AND|WITH)[ \t]+{IDENT})*)"
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


#: An SPDX identifier expression as `reuse` would extract one: the tag, a colon or
#: an equals, then a licence-shaped token. Deliberately NOT a bare mention - the
#: gates that talk ABOUT the tag (`check-image-licensing.py`, this file's own
#: `HEADER` pattern) name it without a value and are not declarations.
EXPRESSION = re.compile(r'SPDX-License-Identifier\s*[:=]\s*"?([A-Za-z0-9][A-Za-z0-9.\-+]*)')

#: Files whose every expression IS a declaration, so the rule does not apply.
DECLARATION_FILES = {"REUSE.toml"}

#: The markers `reuse` honours around text that is not a declaration.
IGNORE_START = "REUSE-Ignore" + "Start"


def unmarked_fixture_licences(path: pathlib.Path, rel: str) -> int:
    """How many licence expressions a file carries beyond its own header, unmarked.

    Zero for almost every file. A test that plants sample headers gets a count,
    and the fix is the ignore-marker pair around the samples - not deleting them.
    """
    if rel in DECLARATION_FILES:
        return 0
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return 0
    found = list(EXPRESSION.finditer(text))
    if not found:
        return 0
    if IGNORE_START in text:
        return 0
    # The first match inside the header window is the file's own declaration; the
    # same window `own_licence` reads, so the two rules agree about what a header is.
    own = 1 if found[0].start() < 4096 else 0
    return len(found) - own


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

    checked, bad, unmarked = 0, [], []
    for rel in tracked(ROOT):
        extra = unmarked_fixture_licences(ROOT / rel, rel)
        if extra:
            unmarked.append(
                f"{rel}: carries {extra} licence expression(s) beyond its own header, "
                f"unmarked.\n    `reuse lint` reads them as this file's declaration. "
                f"Wrap the samples in REUSE-Ignore" + "Start / REUSE-Ignore" + "End."
            )
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

    for b in bad + unmarked:
        print(b)
    if bad or unmarked:
        print(
            f"\n{len(bad)} file(s) disagreeing with the licence map, "
            f"{len(unmarked)} carrying an unmarked fixture licence"
        )
        return 1
    print(
        f"check-license-headers-agree: {checked} headers agree with the map, "
        f"{len(KNOWN)} recorded carve-outs, no unmarked fixture licence"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
