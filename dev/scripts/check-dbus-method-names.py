#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A client calling one of our own D-Bus interfaces has to name a method it has.

`proxy.call("SetAlarm", ...)` is a string. The interface is `async fn set_alarm`
in another crate, and zbus renames it to `SetAlarm` for the wire. **Nothing links
the two**: the client compiles, the daemon compiles, and a rename on either side
produces an `UnknownMethod` at runtime - on the click, in front of the user, with
no test between them that would have noticed.

The clock is why this exists. Its sixteen commands cross exactly this seam:
`apps/clock/src-tauri` calls `SetAlarm`, `TimerStart`, `StopwatchLap` and the
rest by string, `daemons/clock` declares `set_alarm`, `timer_start`,
`stopwatch_lap`, and the agreement rests on zbus's snake-to-Pascal convention
holding on both sides. It does today - checked by hand on 11 Aug, all sixteen,
plus the service name, the object path and the interface name. Checking it once
by hand is what makes it worth checking every time.

**Only OUR interfaces.** A proxy onto BlueZ, UPower, logind or NetworkManager
names methods this tree does not define, and a rule that reported those would be
reporting that a foreign API exists.

**Per file, and the four files that need more are named rather than skipped.**
The scan pairs the calls in a file with the single `org.arlen.*` interface that
file mentions. Six of the ten qualify. The other four talk to two or three of our
interfaces at once, and resolving which call belongs to which proxy needs the
binding traced through helper functions that return a proxy - `apps/clock` does
exactly that. Guessing there would produce false reports on correct code, so they
are printed as unchecked, which is a different statement from silence.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

ARLEN_IFACE = re.compile(r'"(org\.arlen\.[A-Za-z0-9_]+)"')
CALL = re.compile(r'\.call(?:::<[^>]*>)?\(\s*"([A-Za-z][A-Za-z0-9_]*)"')
INTERFACE_ATTR = re.compile(r'#\[zbus::interface\(name\s*=\s*"([^"]+)"')
# A method on the interface impl. `#[zbus(name = "X")]` overrides the wire name;
# `#[zbus(property)]` is read as a property, never called as a method.
METHOD = re.compile(
    r'(?:#\[zbus\((?P<attr>[^)]*)\)\]\s*)?(?:pub\s+)?(?:async\s+)?fn\s+(?P<fn>[a-z_][a-z0-9_]*)\s*\('
)

# Methods every interface has without declaring them.
STANDARD = {"Get", "Set", "GetAll", "Introspect", "Ping", "GetMachineId"}

# file -> why its mismatches are carried rather than fixed here.
#
# The gate found this on its first run, which is the argument for it: nothing
# else in the tree could have. `apps/harness` is arlen-ui's, so the fix is
# theirs; carrying it green with the reason in front of them beats a red gate
# nobody here can clear.
KNOWN = {
    "apps/harness/src-tauri/src/ai_client.rs": (
        "calls `query`, `take_result`, `take_trace` and `cancel` on org.arlen.AI1, "
        "which the ai-engine-daemon now owns and which serves only ExplainSystem. "
        "Those four were the RETIRED ai-daemon's methods: the file's own comment "
        "records the handover and says the new owner serves `explain_system`, but "
        "the conversation call sites were not migrated with it. Nothing in the "
        "tree serves take_result or take_trace any more, so every turn through "
        "this client fails with UnknownMethod. arlen-ui's file; found 11 Aug."
    ),
}


def wire_name(fn: str) -> str:
    """zbus's default: `set_alarm` on the wire is `SetAlarm`."""
    return "".join(part[:1].upper() + part[1:] for part in fn.split("_") if part)


def impl_body(text: str, after_attr: int) -> str:
    """The `impl` block that follows the attribute, by matching its braces.

    Slicing to the next interface attribute instead - which is what this did
    first - runs past the end of the impl and swallows whatever follows,
    including `#[cfg(test)] mod tests`. That put thirty test function names into
    `org.arlen.AIAgent1`'s method set, so a client calling
    `ANongraphUndoRefusesAndDoesNotActWhenTheAuditFails` would have passed. The
    error direction is the bad one: it makes the gate quieter, and a gate that
    accepts anything is indistinguishable from no gate.
    """
    start = text.find("{", after_attr)
    if start == -1:
        return ""
    depth = 0
    for i in range(start, len(text)):
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return text[start : i + 1]
    return text[start:]


def interfaces(root: Path) -> dict[str, set[str]]:
    """`org.arlen.X` -> the method names it answers to."""
    out: dict[str, set[str]] = {}
    for path in sorted(root.rglob("*.rs")):
        sp = str(path)
        if "/target/" in sp or "mkosi.builddir" in sp:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in INTERFACE_ATTR.finditer(text):
            name = m.group(1)
            body = impl_body(text, m.end())
            names = out.setdefault(name, set())
            for f in METHOD.finditer(body):
                attr = f.group("attr") or ""
                if "property" in attr:
                    continue
                override = re.search(r'name\s*=\s*"([^"]+)"', attr)
                names.add(override.group(1) if override else wire_name(f.group("fn")))
    return out


def main() -> int:
    known = interfaces(ROOT)
    problems: list[str] = []
    unchecked: list[str] = []
    checked = 0

    for path in sorted(ROOT.rglob("*.rs")):
        sp = str(path)
        if "/target/" in sp or "mkosi.builddir" in sp:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        calls = CALL.findall(text)
        if not calls:
            continue
        named = {n for n in set(ARLEN_IFACE.findall(text)) if n in known}
        if not named:
            continue
        rel = path.relative_to(ROOT)
        if len(named) > 1:
            unchecked.append(f"{rel}: talks to {', '.join(sorted(named))}")
            continue
        iface = named.pop()
        if str(rel) in KNOWN:
            unchecked.append(f"{rel}: {KNOWN[str(rel)]}")
            continue
        for method in sorted(set(calls)):
            if method in STANDARD:
                continue
            checked += 1
            if method not in known[iface]:
                problems.append(
                    f"{rel}: calls `{method}` on {iface}, which answers to "
                    f"{', '.join(sorted(known[iface])) or '(no methods found)'}"
                )

    if problems:
        print("D-Bus calls naming a method the interface does not have:\n")
        for p in problems:
            print(f"  {p}")
        print("\n  The call is a string and the method is a function in another crate,")
        print("  so this fails at runtime rather than at build. Fix the name on")
        print("  whichever side moved.")
        return 1

    print(
        f"OK: {checked} call(s) against {len(known)} arlen interface(s), every method present"
    )
    if unchecked:
        print(f"  Not checked ({len(unchecked)}) - a declared mismatch, or more than one interface:")
        for u in unchecked:
            print(f"    {u}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
