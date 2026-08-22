#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that the consent surface does not take input at the moment it is mapped.

The rule is the planner's, in `consent-grant-surface-plan.md`'s do-not list: map,
paint, then take input - however long the paint takes. A surface that cannot paint
yet holds no input and answers nothing.

It is here because prose does not fail a build, and this exact regression is one
line. On 22 August a click driven 1.1 seconds after the raise was taken as the
answer while the frame captured at that instant showed a plain desktop: no card,
nothing to read. Consent taken from somebody who could not see the question is not
consent, and the click landed on Allow.

What it checks: `show()` in the consent window must not arm the surface. Arming is
a full input region (`input_shape_combine_region` over a whole-screen rectangle)
or an exclusive keyboard mode. Only the separate `arm()` may do either, and that
is called by the card's own component once it has laid out.

NECESSARY, NOT SUFFICIENT, and the number matters so nobody reads a green run as a
closed hole: with this shape in place `arm` fires 37ms after the raise while the
card reaches the screen a second or more later, and a click at raise+1.2s was
still taken as the answer. The page's readiness report is a rAF pair, and rAF
completes as soon as GTK ticks a frame - which it does in 4ms while WebKit's
pixels are seconds away.

So this holds the SHAPE: mapped without input, armed from one place. Whether the
report that arms it is honest about pixels is measured on the image, not here, and
today it is not. Three signals have been measured and none of them means pixels.

Deliberately narrow. It names one surface rather than every layer window, because
the top bar legitimately takes input the moment it maps: it has no question to
read.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

SURFACE = "apps/desktop-shell/src-tauri/src/consent_window.rs"

# Arming, in the two spellings that exist.
FULL_REGION = re.compile(r"RectangleInt::new\(\s*0\s*,\s*0\s*,\s*(\d+)\s*,\s*(\d+)\s*\)")
EXCLUSIVE = re.compile(r"KeyboardMode::Exclusive")


def body_of(text: str, name: str) -> str | None:
    """The body of `pub fn <name>`, by brace matching."""
    m = re.search(rf"pub fn {re.escape(name)}\s*\([^)]*\)[^{{]*{{", text)
    if not m:
        return None
    depth = 0
    for i in range(m.end() - 1, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[m.end() : i]
    return None


def main() -> int:
    path = ROOT / SURFACE
    if not path.is_file():
        print(f"NOTHING WAS READ: no consent surface at {path}", file=sys.stderr)
        return 2

    text = path.read_text(encoding="utf-8", errors="replace")
    show = body_of(text, "show")
    arm = body_of(text, "arm")

    problems: list[str] = []
    if show is None:
        problems.append(
            "the consent window has no `show`, so this check cannot see the moment "
            "the surface is mapped. If it was renamed, rename it here too rather "
            "than leaving a check that reads nothing."
        )
    else:
        for m in FULL_REGION.finditer(show):
            if int(m.group(1)) > 1 and int(m.group(2)) > 1:
                problems.append(
                    f"`show` arms a {m.group(1)}x{m.group(2)} input region at the "
                    f"moment it maps the surface, so a press lands on a card that "
                    f"has not painted. Map with an empty region and let `arm` do "
                    f"this when the card reports it is up."
                )
        if EXCLUSIVE.search(show):
            problems.append(
                "`show` takes the keyboard exclusively at the moment it maps the "
                "surface. A key aimed at whatever was on screen answers a question "
                "nobody has read yet."
            )

    if arm is None:
        problems.append(
            "there is no `arm`, so nothing can make the surface answerable after it "
            "has painted - a consent surface that never takes input is safe and "
            "useless, which is not the rule either."
        )

    if problems:
        print("the consent surface takes input before it has painted:")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        "the consent surface maps holding no input; only `arm` makes it "
        "answerable. THE SHAPE ONLY: on 22 August `arm` fired 37ms after the raise "
        "while the card reached the screen a second later, and a click in that gap "
        "was still taken as the answer. A green run here is not a closed hole."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
