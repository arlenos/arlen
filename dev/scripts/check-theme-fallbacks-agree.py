#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every app's first-paint colour fallbacks are the shipped theme's.

Each app carries its own `app.css` with a `:root` block of surface tokens, and
each block says in its own comment that it mirrors `sdk/theme/themes/dark.toml`
"for first-paint correctness before the runtime theme injection runs". Eighteen
copies of one palette, and until 15 September nothing compared them: sixteen had
`--color-border: #262626` where the theme says `#27272a`, sixteen had a different
green for `--color-success` (`#10b981` against `#22c55e`), and sixteen defined no
`--color-info` at all - so a component reaching for it before injection got
nothing.

They are fallbacks, so the drift is invisible whenever the theme lands. It is not
invisible when it does not: the greeter is the surface a first-run reader judges
the system by and it is also the one that has shipped without its locale init
before, and a window that paints for a moment in the wrong green is the mildest
of the failures this shape produces.

    drifted    a literal that differs from the theme's
    missing    a token the theme defines and a copy does not name

Only the DARK `:root` block is compared. A light block below it legitimately
redefines some of these, and this gate stops at the first definition of each -
which is the dark one.
"""

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
THEME = ROOT / "sdk/theme/themes/dark.toml"

# The CSS token each theme field is copied into. Only literals are compared: a
# copy is free to write `var(--color-fg-primary)` where the theme names a colour,
# which several do for the monochrome accent, and that is a statement about the
# relationship rather than a stale number.
TOKENS = {
    "--color-bg-shell": ("color.bg", "shell"),
    "--color-bg-app": ("color.bg", "app"),
    "--color-bg-card": ("color.bg", "card"),
    "--color-bg-overlay": ("color.bg", "overlay"),
    "--color-bg-input": ("color.bg", "input"),
    "--color-fg-primary": ("color.fg", "primary"),
    "--color-fg-secondary": ("color.fg", "secondary"),
    "--color-fg-disabled": ("color.fg", "disabled"),
    "--color-fg-inverse": ("color.fg", "inverse"),
    "--color-border-strong": ("color.border", "strong"),
    "--color-error": ("color.semantic", "error"),
    "--color-warning": ("color.semantic", "warning"),
    "--color-success": ("color.semantic", "success"),
    "--color-info": ("color.semantic", "info"),
}

# `--color-border` is deliberately absent from the table above: several copies
# define it as `var(--color-border-default)` so a light block can move both at
# once, and comparing a var against a literal would report a design as a defect.
# Its literal, where a copy writes one, is checked through `--color-border-default`
# when that exists.
TOKENS_EITHER = {
    ("--color-border", "--color-border-default"): ("color.border", "default"),
}

# Copies this gate reads but does not hold to the rule, with the reason.
# MAY SHRINK, MAY NOT GROW.
CARRIED = {
    "apps/harness/src/app.css": (
        "arlen-ui's live work; the drift is the same two values and the same "
        "missing token as everywhere else, written up for them rather than "
        "changed underneath them"
    ),
    "apps/store/src/app.css": ("arlen-ui's, same as the harness"),
}


def theme_values(text: str) -> dict[tuple[str, str], str]:
    """Every `[table] key = "value"` in the theme file, lowercased."""
    out: dict[tuple[str, str], str] = {}
    table = ""
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("[") and line.endswith("]"):
            table = line[1:-1]
            continue
        m = re.match(r'^(\w+)\s*=\s*"([^"]*)"', line)
        if m:
            out[(table, m.group(1))] = m.group(2).lower()
    return out


def first_definition(css: str, token: str) -> str | None:
    """The first value given to `token`, which is the dark block's."""
    m = re.search(r"^\s*%s:\s*([^;]+);" % re.escape(token), css, re.M)
    return m.group(1).strip().lower() if m else None


def check(path: Path, css: str, want: dict[tuple[str, str], str]) -> list[str]:
    rel = path.relative_to(ROOT)
    problems = []
    for token, key in TOKENS.items():
        expected = want.get(key)
        if expected is None:
            return [f"{rel}: the theme defines no {key[0]}.{key[1]}, so nothing can be compared"]
        got = first_definition(css, token)
        if got is None:
            problems.append(f"{rel}: {token} is not defined; the theme says {expected}")
        elif got.startswith("#") and got != expected:
            problems.append(f"{rel}: {token} is {got}, the theme says {expected}")
    for tokens, key in TOKENS_EITHER.items():
        expected = want.get(key)
        literals = [first_definition(css, t) for t in tokens]
        literals = [v for v in literals if v and v.startswith("#")]
        if literals and expected and literals[0] != expected:
            problems.append(
                f"{rel}: {tokens[0]} resolves to {literals[0]}, the theme says {expected}"
            )
    return problems


def main() -> int:
    if not THEME.is_file():
        print(f"!! NOTHING WAS READ: no theme file at {THEME}", file=sys.stderr)
        return 2
    want = theme_values(THEME.read_text(encoding="utf-8", errors="replace"))
    if not want:
        print(f"!! NOTHING TO COMPARE AGAINST: {THEME} holds no key/value pairs", file=sys.stderr)
        return 2

    copies = sorted(ROOT.glob("apps/*/src/app.css")) + [ROOT / "sdk/ui-kit/src/app.css"]
    copies = [p for p in copies if p.is_file()]
    if not copies:
        print("!! NOTHING WAS READ: no app.css found under apps/ or sdk/ui-kit", file=sys.stderr)
        return 2

    problems, carried = [], 0
    for path in copies:
        rel = str(path.relative_to(ROOT))
        if rel in CARRIED:
            carried += 1
            continue
        problems += check(path, path.read_text(encoding="utf-8", errors="replace"), want)

    if problems:
        print("A first-paint fallback disagrees with the theme it says it mirrors:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        print(
            "\nCopy the theme's value. If the difference is deliberate, the theme is the\n"
            "place to change it, so every copy follows.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{len(copies) - carried} app.css fallback block(s) agree with the shipped dark theme"
        f"{f', {carried} carried' if carried else ''}."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
