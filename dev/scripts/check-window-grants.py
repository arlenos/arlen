#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a window permission an app grants is one its webview actually calls.

Tauri capabilities gate what the WEBVIEW may invoke. A grant with no call behind
it is authority nobody uses, which is the least-privilege half of the same
question `check-granted-and-used.py` asks - except that check bites on exactly two
permissions, the ones with a single canonical consumer, so every
`core:window:allow-*` grant in the tree was unchecked.

Those can be checked without guessing, which is why this exists and stops here:
the permission name IS the method name in Tauri's own vocabulary
(`allow-unmaximize` to `.unmaximize()`). No mapping was invented. Anything whose
call site is not derivable that way stays out, on the rule the opener gate states:
a check that guesses is worse than one that admits its scope.

**A shared component's calls count as the app's**, and getting this wrong is the
whole reason the first measurement was useless. `WindowControls` in the ui-kit
calls close, minimize, maximize, unmaximize and isMaximized; an app that renders
it needs those grants and makes no such call in its own `src`. Reading only
`apps/*/src` reported thirteen apps over-granting, uniformly, which is the
signature of a shared caller rather than thirteen mistakes. An app that imports
the control is credited with what the control calls.

What this does NOT cover:

  * a Rust-side `window.show()`. It needs no grant - capabilities gate the
    webview - so it is correctly not evidence that a grant is used.
  * a call built at runtime from a variable, the same blind spot every scanner
    here has.
  * `data-tauri-drag-region` is NOT a gap - it needs
    `allow-start-dragging` with no method call at all, so it IS a call site and is
    read as one. This line first said the attribute occurs once in the tree, inside
    `WindowControls` - it occurs in nine app files, and the sentence came from the
    same broken pathspec that produced the six-window-references claim above.
    Treating it as a call is not a guess: it is the documented way to invoke that
    permission.
  * whether a grant that IS called is one the app should have. This finds
    authority with nothing behind it, not authority that is too much.
  * permissions outside `core:window:`. `core:default` and `core:event:default`
    have no name-to-call rule at all. Arlen's own plugin permissions DO -
    `arlen-menu:allow-set-title` names the command a webview invokes as
    `plugin:arlen-menu|set_title` - and widening to them was tried on 12 Aug and
    backed out. All 22 are backed by a call today, checked by hand, so it fixed
    nothing; and it reported four that are not real, because those calls come from
    ui-kit modules other than `WindowControls` and the credit above is gated on
    that one component. The shared-caller question has to be answered again for
    each family, and answering it for plugin commands needs per-component
    attribution rather than one import check. Left out rather than loosened: a
    credit wide enough to cover any kit call would pass every app for a call one
    app makes.

Run: dev/scripts/check-window-grants.py [tree]
"""

import json
import re
import sys
from pathlib import Path

# The tree to scan. An argument so this can be pointed at a fixture and shown to
# fail (standing rule, 11 Aug).
ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# Tauri's own naming, not a mapping somebody chose. Extend it only for a
# permission whose method name is derivable the same way.
METHOD = {
    "close": "close",
    "hide": "hide",
    "show": "show",
    "minimize": "minimize",
    "maximize": "maximize",
    "unmaximize": "unmaximize",
    "is-maximized": "isMaximized",
    "start-dragging": "startDragging",
    "set-title": "setTitle",
    "set-focus": "setFocus",
}

GRANT = re.compile(r"core:window:allow-(.+)")

# The queue, worst first, with the count that keeps it a queue rather than a hole:
# an app that grows a NEW unused grant fails even though it is listed.
#
# It started at seventeen across twelve apps and is five across two, because the
# `allow-show` twelve were removed rather than carried (see below). What is left is
# genuinely two apps: `screenshot`, which renders no window control and grants four
# it never calls, and `store`'s `hide`. Both are one-app decisions rather than a
# template, so they want somebody who can watch that window rather than a sweep.
KNOWN: dict[str, tuple[int, str]] = {
    "screenshot": (4, "close, hide, minimize, start-dragging - it renders no window control"),
    "store": (1, "hide; arlen-ui's surface, the capability file is not"),
}

# Twelve `core:window:allow-show` grants left this list on 12 Aug by being removed
# from the capability files, not excused: none of those twelve apps calls `.show()`
# in its frontend, its Rust, or any kit code it renders, and a permission with no
# reachable caller cannot break one when it goes.
#
# **The first version of that reasoning was wrong and is worth keeping wrong here.**
# It said the whole webview layer held six window references, all in the two
# window-control files - so nothing anywhere could call show. That came from
# `git grep ... -- 'apps/*/src'`, whose pathspec silently under-matched: the real
# number is EIGHTEEN files, and `.show()` does exist, in
# `apps/settings/src/routes/+layout.svelte`. Settings was not one of the twelve and
# still grants what it calls, so the removal holds - but it held for a narrower
# reason than the one written down, and a right answer resting on a false premise
# is one edit away from a wrong one. Re-derived by walking the tree in Python
# rather than trusting a glob.
#
# The count-drop guard reported all twelve the moment they emptied, which is what
# it was added for two hours earlier.


def kit_calls() -> str:
    """The shared ui-kit's source, whose window calls an importing app inherits."""
    out = []
    kit = ROOT / "sdk/ui-kit/src"
    if kit.is_dir():
        for f in list(kit.rglob("*.svelte")) + list(kit.rglob("*.ts")):
            try:
                out.append(f.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                continue
    return "\n".join(out)


def granted(app: Path) -> set[str]:
    """The permission identifiers this app's capability files declare."""
    out: set[str] = set()
    for c in (app / "src-tauri").rglob("capabilities/*.json"):
        try:
            doc = json.loads(c.read_text(encoding="utf-8", errors="replace"))
        except (OSError, json.JSONDecodeError):
            continue
        out |= {p for p in doc.get("permissions", []) if isinstance(p, str)}
    return out


def frontend(app: Path, kit: str) -> str:
    """The app's own frontend source, plus the kit's if it renders the control."""
    parts = []
    src = app / "src"
    if src.is_dir():
        for f in list(src.rglob("*.ts")) + list(src.rglob("*.svelte")):
            try:
                parts.append(f.read_text(encoding="utf-8", errors="replace"))
            except OSError:
                continue
    text = "\n".join(parts)
    if "WindowControls" in text or "window-controls" in text:
        text += kit
    return text


def main() -> int:
    apps = sorted(p for p in (ROOT / "apps").glob("*") if (p / "src-tauri").is_dir())
    if not apps:
        # A count of zero is only honest when there was something to count.
        print("found no Tauri apps under apps/; the layout moved and this check went quiet")
        return 1

    kit = kit_calls()
    findings: list[str] = []
    seen: dict[str, int] = {}
    checked = 0

    for app in apps:
        text = frontend(app, kit)
        for g in sorted(granted(app)):
            m = GRANT.fullmatch(g)
            if not m or m.group(1) not in METHOD:
                continue
            checked += 1
            if f".{METHOD[m.group(1)]}(" in text:
                continue
            # The attribute form. `data-tauri-drag-region` invokes start-dragging
            # with no method call, so a file carrying it IS a call site.
            if m.group(1) == "start-dragging" and "data-tauri-drag-region" in text:
                continue
            seen[app.name] = seen.get(app.name, 0) + 1
            if app.name in KNOWN and seen[app.name] <= KNOWN[app.name][0]:
                continue
            findings.append(
                f"apps/{app.name}: grants `{g}` and no call to "
                f"`.{METHOD[m.group(1)]}(` reaches it - authority with nothing "
                f"behind it. Drop the grant, or make the call the app needs."
            )

    # Both directions, the rule four other lists here learned on 12 Aug. Scoped to
    # this tree, because KNOWN is a set of counts about one repo and a fixture
    # lacks these apps for reasons that have nothing to do with the entries.
    own_tree = len(sys.argv) <= 1 or ROOT == Path(__file__).resolve().parents[2]
    for name, (declared, _) in sorted(KNOWN.items() if own_tree else []):
        found = seen.get(name, 0)
        if found < declared:
            findings.append(
                f"apps/{name}: carried as {declared} unused grant(s) and only "
                f"{found} remain. Lower the number, or drop the entry at zero - a "
                f"count that is too high leaves room for a new one to go unseen."
            )

    print(
        f"{checked} window grant(s) across {len(apps)} app(s) checked for the call "
        f"that uses them. A shared control's calls count as the app's, which is "
        f"what separates a real over-grant from an app that renders WindowControls."
    )
    if findings:
        print("\ngrants with nothing behind them:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    carried = sum(n for n, _ in KNOWN.values())
    print(f"{carried} carried in {len(KNOWN)} app(s), each bounded by its recorded count.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
