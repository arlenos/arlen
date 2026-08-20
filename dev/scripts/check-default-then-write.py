#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A read defaulted to empty, then a write of the whole thing, is an erase.

    let existing = read(path).unwrap_or_default();   // unreadable becomes empty
    ...modify...
    write(path, everything);                          // and empty goes back

The default reads as caution. It is the opposite: the function has decided that a
file it could not read holds nothing, and then written that decision to disk. The
person's data is gone and the call returns Ok.

Every one of these was on a file somebody owns, and the damage was never
proportional to the action that triggered it:

    knowledge_search_save        one corrupt byte, and the next save renames a
                                 one-entry list over every saved search
    update_layout_field          one syntax error in `compositor.toml` plus one
                                 drag of the gaps slider, and every keybinding,
                                 window rule and system action is replaced by a
                                 four-line `[layout]` block
    write_toml_key               the same, and `settings_set_value` calls it, so
                                 it reaches every config the shell writes
    register_default_handler     `mimeapps.list` holds which application opens a
                                 PDF, a spreadsheet, a video - for every app on
                                 the machine, not just ours
    modulesd::persist            `modules = []` over the disabled list silently
                                 re-enables every module the person switched off

The distinction that fixes all five is the same one: **absent is not unreadable.**
A missing file means nobody has configured anything, so an empty value is the
truth and the write creates it. Any other read error means the answer is unknown,
and a write that assumes it destroys what it could not see.

ONE HOP IS FOLLOWED NOW, and that is not a refinement - it is the hole a real
erase went through. `apps/files` read its config in a helper
(`read_files_config`, ending in `.unwrap_or_default()`) and did the
read-modify-write in three commands that called it, so the load and the write were
never in one body and this check said nothing while a corrupt `files.toml` plus one
pinned folder would erase every smart folder. A same-file helper whose whole job is
that defaulted read is now counted as the load, which is the same widening
`check-optimistic-write` needed for the same reason: a gate matches the shape its
author last happened to write.

WHAT IT STILL CANNOT SEE: a loader two hops away, a helper in another file, and a
write through a type that owns its own persistence. It caught five by shape and a
sixth by this hop, and cried wolf zero times over the tree, which is the ratio that
decides whether a check gets read or switched off. A partial check that never lies
is worth more than a total one nobody trusts.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

ROOTS = ("apps", "daemons", "sdk", "ai", "contracts", "forage")
SKIP = ("/target/", "node_modules", "/.git/", "mkosi.builddir")

# A read whose failure collapses into an empty value.
LOAD = re.compile(
    r"(?:read_to_string|read|load|from_str|from_slice)\s*"
    r"(?:::<[^>]*>)?\s*\([^;]{0,240}?\)"
    r"(?:\.await)?\s*"
    r"(?:\.map\([^;]{0,120}?\))?\s*"
    r"\.(?:unwrap_or_default\(\)"
    r"|unwrap_or_else\(\s*\|_\|[^;]{0,80}?\)"
    r"|unwrap_or\(\s*(?:Vec::new\(\)|vec!\[\]|String::new\(\)|Default::default\(\)"
    r"|BTreeSet::new\(\)|HashSet::new\(\)|HashMap::new\(\)|BTreeMap::new\(\))\s*\))",
    re.S,
)

# A write of the whole file, after it.
WRITE = re.compile(r"fs::write|fs::rename|write_all|to_vec_pretty|to_string_pretty\s*\(")

FN = re.compile(r"fn\s+(\w+)\s*[(<]")

# A same-file helper whose body is the defaulted read. Its callers are then read as
# if the load were written out in them, which is where the files-config erase hid.
FILE_READ = re.compile(r"(?:fs::)?read_to_string\s*\(|fs::read\s*\(")


def defaulting_loaders(text: str) -> set[str]:
    """Names of same-file fns whose body is a FILE read defaulted to empty.

    The file read is the part that matters, and requiring it is what keeps this
    from crying wolf. A first cut asked only for the defaulted-parse shape, and
    counted `parse_disabled(text)` - a pure parser taking a string somebody else
    read - as a loader, which made `modulesd::persist` look like an erase when it
    is the model answer, NotFound told apart from unreadable with the reasoning
    written above it. A helper that does not open the file cannot be the read this
    check is about.
    """
    out: set[str] = set()
    for m in FN.finditer(text):
        body = body_of(text, m.end())
        # Bounded on purpose: a helper that does the defaulted read AND other work
        # is not the shape this follows, and calling it is not evidence of a load.
        if (
            body
            and len(body) < 900
            and FILE_READ.search(body)
            and LOAD.search(body)
            and not WRITE.search(body)
        ):
            out.add(m.group(1))
    return out


def writing_helpers(text: str) -> set[str]:
    """Names of same-file fns that write a whole file when called.

    The other half of the hop, and without it the first half is useless: the erase
    this was widened for called `read_files_config()` and `write_files_config()`,
    so the caller's body contained neither an `fs::write` nor a defaulted read. A
    first cut followed only the loader and still said nothing, which is the sort of
    half-fix that leaves a check looking like cover.
    """
    out: set[str] = set()
    for m in FN.finditer(text):
        body = body_of(text, m.end())
        if body and len(body) < 900 and WRITE.search(body) and not LOAD.search(body):
            out.add(m.group(1))
    return out

# Sites carried with a reason. An entry says why the erase is not one - not that
# somebody looked at it once.
KNOWN: dict[str, str] = {}


def body_of(text: str, start: int) -> str:
    """The braced body beginning at or after `start`."""
    i = text.find("{", start)
    if i < 0:
        return ""
    depth = 0
    for j in range(i, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[i : j + 1]
    return text[i:]


def sources(repo: Path):
    for root in ROOTS:
        base = repo / root
        if not base.is_dir():
            continue
        for f in base.rglob("*.rs"):
            if not any(s in str(f) for s in SKIP):
                yield f


def main() -> int:
    files = list(sources(REPO))
    if not files:
        print(f"NOTHING WAS READ: no Rust sources under {REPO}", file=sys.stderr)
        return 2

    problems: list[str] = []
    carried: list[str] = []
    scanned = 0

    for f in sorted(files):
        text = f.read_text(encoding="utf-8", errors="replace")
        # A test that writes a fixture and reads it back is not an erase, and the
        # hop below reaches into test bodies the shape match never did.
        cut = text.find("#[cfg(test)]")
        text = text if cut < 0 else text[:cut]
        loaders = defaulting_loaders(text)
        writers = writing_helpers(text)
        for m in FN.finditer(text):
            body = body_of(text, m.end())
            if not body:
                continue
            scanned += 1
            if m.group(1) in loaders:
                continue  # the helper itself: it does not write
            load = LOAD.search(body)
            if load is None:
                # A call to a same-file helper that does the defaulted read counts
                # as the load, at the position of the call.
                for name in loaders:
                    load = re.search(rf"\b{re.escape(name)}\s*\(", body)
                    if load:
                        break
            if not load:
                continue
            after = body[load.end() :]
            wrote = WRITE.search(after) or next(
                (w for w in writers if re.search(rf"\b{re.escape(w)}\s*\(", after)), None
            )
            if not wrote:
                continue
            rel = f"{f.relative_to(REPO)}:{m.group(1)}"
            if rel in KNOWN:
                carried.append(f"{rel}: {KNOWN[rel]}")
                continue
            line = text[: m.start()].count("\n") + 1
            problems.append(
                f"{f.relative_to(REPO)}:{line} `{m.group(1)}`: a read that failed "
                f"becomes an empty value, and this function writes.\n"
                f"    Tell ABSENT from UNREADABLE: `Err(e) if e.kind() == NotFound` "
                f"is the empty case and may write; any other error must refuse and "
                f"leave the file alone, so a person can still fix it by hand."
            )

    if OWN_TREE:
        for rel in sorted(KNOWN):
            path = REPO / rel.split(":")[0]
            if not path.is_file():
                problems.append(f"{rel}: carried, and the file is gone. Drop the entry.")

    if carried:
        print("carried, with a reason (see KNOWN):")
        for line in carried:
            print(f"  {line}")
        print()

    if problems:
        print("a defaulted read feeding a write:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"OK: {scanned} function(s) checked; none writes a file back from a read it "
        f"could not perform"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
