#!/usr/bin/env python3
"""Every locale block in a catalogue defines the same keys.

WHY THIS EXISTS BESIDE `check-message-keys`, which already checks both locales.
That one is deliberately one-directional: it reports a key the code ASKS FOR and
the catalogue lacks, and refuses to report the reverse, because 47 files in this
tree build keys at runtime (`$t(error)`, `$t(tb.id)`) and about nine hundred keys
are reached that way. Its own header says why the restraint is load-bearing:
reporting them would invite deleting a string somebody's error path needs.

The consequence is a gap, and it is the one this closes. A key that is reached
DYNAMICALLY and added to `en` but not `de` is asked for by no literal, so the
other check never looks at it, and the German build renders the key or falls back
to English forever. That is the shape this project has already shipped once: the
greeter carried a whole German catalogue that nothing could reach, and it was
found by rendering the login screen at `?locale=de`, not by a check.

This needs no cross-referencing at all - a locale block either has the key or it
does not - so unlike a dead-key scan it is decidable, and unlike its sibling it
cannot invite a deletion: the fix for a finding here is always to ADD the missing
translation.

WHAT IT CANNOT SAY: whether a translation is a translation. A `de` entry holding
the English sentence passes, and only a person reading it, or a render at
`?locale=de`, catches that.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]

# `  en: {` at the top level of a catalogue object.
LOCALE = re.compile(r'^\s{2}([a-z]{2}(?:-[A-Za-z]+)?)\s*:\s*\{')
# `    "a.b.c": "…"` - a key definition inside one.
KEY = re.compile(r'^\s*"([a-zA-Z][\w.]*)"\s*:')


def catalogues() -> list[pathlib.Path]:
    """Every message catalogue under ROOT, including the kit's and the portal's."""
    try:
        out = subprocess.run(
            ["git", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
        ).stdout.split()
        return sorted(ROOT / f for f in out if "/i18n/messages" in f and f.endswith(".ts"))
    except (subprocess.CalledProcessError, FileNotFoundError):
        return sorted(ROOT.rglob("*/i18n/messages*.ts"))


def blocks(path: pathlib.Path) -> dict[str, set[str]]:
    """The keys each locale block in one file defines."""
    out: dict[str, set[str]] = {}
    current: str | None = None
    for line in path.read_text(encoding="utf-8").splitlines():
        m = LOCALE.match(line)
        if m:
            current = m.group(1)
            out.setdefault(current, set())
            continue
        k = KEY.match(line)
        if k and current:
            out[current].add(k.group(1))
    return out


def main() -> int:
    """Refuse a catalogue whose locales disagree about which keys exist."""
    cats = catalogues()
    if not cats:
        print("check-locale-parity: no catalogue found, which is itself wrong")
        return 1
    findings: list[str] = []
    pairs = 0
    for path in cats:
        by = blocks(path)
        if len(by) < 2:
            continue
        locales = sorted(by)
        base = locales[0]
        for other in locales[1:]:
            pairs += 1
            for missing_in, present_in in ((other, base), (base, other)):
                gap = sorted(by[present_in] - by[missing_in])
                if gap:
                    findings.append(
                        f"{path.relative_to(ROOT)}: {len(gap)} key(s) in `{present_in}` "
                        f"and not in `{missing_in}` - {', '.join(gap[:5])}"
                        f"{', …' if len(gap) > 5 else ''}. A key one locale lacks renders "
                        f"as the key or as the other language, and if nothing asks for it "
                        f"by a literal name then no other check here will see it."
                    )
    for f in findings:
        print(f)
    if findings:
        print(f"\n{len(findings)} locale gap(s) across {len(cats)} catalogue(s)")
        return 1
    print(
        f"{len(cats)} catalogue(s), {pairs} locale pair(s), each defining the same keys. "
        f"Presence only: a `de` entry holding an English sentence passes this, and only "
        f"a reader or a render at `?locale=de` catches that."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
