# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a file's writer and its separate reader still agree on the fields.

Some on-disk files are written by one crate and read by another, and because the
two cannot share a type, each declares its own. `installed.lock` is written by
installd's `LockEntry` and read by store-backend's `InstalledEntry`, which carries
three of the eight fields; serde ignores the rest, so a reader taking a subset is
fine and deliberate.

What is not fine is the writer renaming or dropping a field the reader needs.
Nothing errors: `toml::from_str` fails, `parse_lock` returns an empty vec by
design (a store that will not open is worse than one that misses an update), and
the update surface simply goes quiet. Every test on both sides still passes,
because each side is consistent with itself.

This is the same shape as the eight copies of `event.proto`, and the same
treatment: compare the two declarations directly, because the compiler never
will. A reader field with no writer field of that name is the failure; the
reverse is the normal case.
"""

import os
import pathlib
import re
import subprocess
import sys

# A tree to check may be passed in, which is what lets the gate's own test drive it
# against fixtures; the sibling gates take the same argument.
ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

# (what the file is, writer file, writer struct, reader file, reader struct)
PAIRS = [
    (
        "installed.lock",
        "daemons/installd/installd/src/lock.rs",
        "LockEntry",
        "store-backend/src/discover.rs",
        "InstalledEntry",
    ),
]

FIELD = re.compile(r"^\s*(?:pub )?(\w+)\s*:\s*[^,]+,\s*$", re.M)
RENAME = re.compile(r'rename\s*=\s*"([^"]+)"')


def fields(rel: str, struct: str) -> dict[str, str]:
    """Serde field names of one struct, mapped to the Rust name that produced them."""
    path = ROOT / rel
    if not path.is_file():
        # Symmetric with the missing-struct exit below. A rename used to land here
        # as a traceback, which reads like the gate is broken rather than like the
        # pair it watches has moved.
        sys.exit(f"{rel} is gone; the check names it as {struct}'s home and needs updating")
    text = path.read_text()
    m = re.search(rf"struct {struct}\b[^{{]*{{", text)
    if not m:
        sys.exit(f"could not find `struct {struct}` in {rel}; the check needs updating")
    depth, i = 1, m.end()
    while i < len(text) and depth:
        depth += (text[i] == "{") - (text[i] == "}")
        i += 1
    body = text[m.end() : i - 1]

    # Walked in order, carrying the attributes seen since the last field, so a
    # `#[serde(rename)]` binds to the field it precedes. Searching for a line by
    # its text instead attributes the rename to whichever identical line came
    # first, which reports a real disagreement against the wrong field name.
    out: dict[str, str] = {}
    pending: list[str] = []
    for line in body.split("\n"):
        stripped = line.strip()
        if stripped.startswith("#["):
            pending.append(stripped)
            continue
        fm = FIELD.match(line)
        if not fm:
            if stripped and not stripped.startswith("///") and not stripped.startswith("//"):
                pending.clear()
            continue
        renamed = None
        for attr in pending:
            r = RENAME.search(attr)
            if r:
                renamed = r.group(1)
        pending.clear()
        out[renamed or fm.group(1)] = fm.group(1)
    if not out:
        sys.exit(f"parsed no fields from `{struct}` in {rel}; the check needs updating")
    return out


def protocol_copies() -> tuple[list[str], int, str | None]:
    """The Wayland protocol XMLs the shell and the compositor each vendor.

    They must be byte-identical: each side generates bindings from its own copy,
    so a divergence compiles cleanly on both and fails at runtime, when a request
    carries an argument the other end does not expect. `CLAUDE.md` records the
    sync requirement in prose and nothing enforced it.

    The compositor is a separate repo, which is exactly how the one real wire
    divergence in this system survived (nine `event.proto` copies said `origin`,
    the compositor's said `session_id`, and the drift gate collected its subjects
    with `git ls-files` here). So this reaches across when the checkout is there
    and reports plainly when it is not, rather than counting silence as agreement.

    Every copy is collected rather than the shell's, because the shell is not the
    only vendor: `sdk/tauri-plugin-menu` keeps its own `arlen-titlebar-v1.xml` and
    generates its own client bindings from it, so a version that compared the shell
    against the compositor would have watched two of that protocol's three copies
    and called it covered.
    """
    copies: dict[str, list[pathlib.Path]] = {}
    listed = subprocess.run(
        ["git", "ls-files", "*/protocols/*.xml"],
        cwd=ROOT, capture_output=True, text=True, check=False,
    ).stdout.split()
    for rel in listed:
        copies.setdefault(pathlib.Path(rel).name, []).append(ROOT / rel)

    comp = pathlib.Path(
        os.environ.get("COMPOSITOR_PATH", pathlib.Path.home() / "Repositories/compositor")
    ) / "resources/protocols"
    absent = None
    if comp.is_dir():
        for name in copies:
            other = comp / name
            if other.is_file():
                copies[name].append(other)
    else:
        absent = f"{comp} is absent, so the compositor's protocol copies were not compared"

    problems, checked = [], 0
    for name, paths in sorted(copies.items()):
        if len(paths) < 2:
            continue
        checked += len(paths)
        first = paths[0].read_bytes()
        for other in paths[1:]:
            if other.read_bytes() != first:
                problems.append(
                    f"{name}: {_rel(paths[0])} and {_rel(other)} differ. Each side "
                    "generates bindings from its own, so this builds on both and "
                    "breaks only when the two talk."
                )
    return problems, checked, absent


def _rel(p: pathlib.Path) -> str:
    """Repo-relative where possible, absolute for the separate compositor checkout."""
    try:
        return str(p.relative_to(ROOT))
    except ValueError:
        return str(p)


def main() -> int:
    problems: list[str] = []
    checked = 0
    for what, writer_file, writer_struct, reader_file, reader_struct in PAIRS:
        written = fields(writer_file, writer_struct)
        read = fields(reader_file, reader_struct)
        checked += len(read)
        for name in sorted(set(read) - set(written)):
            problems.append(
                f"{what}: {reader_struct} reads `{name}`, which {writer_struct} does not write. "
                "The read fails silently and the surface goes quiet, so neither side's tests catch it."
            )

    proto_problems, proto_checked, skipped = protocol_copies()
    problems += proto_problems

    if problems:
        print("a file's reader and writer disagree:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(f"{len(PAIRS)} shared file(s), {checked} read field(s), all written; "
          f"{proto_checked} protocol copy/copies identical")
    if skipped:
        print(f"NOT CHECKED: {skipped}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
