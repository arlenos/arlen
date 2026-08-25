# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every D-Bus member an app dials must be one of our daemons actually serves.

`check-invoke-exists` does this for Tauri commands, and the class it catches -
a surface wired to something nobody implements - turns out to have a second
home nothing was watching. A Tauri command can exist, compile, be registered,
and still do nothing, because the D-Bus member IT dials does not exist.

That is not hypothetical. `ai_behaviours` dialled `list_skills` on
`org.arlen.AIAgent1`, which serves no such member, and substituted an empty
array on failure - so the Settings behaviours panel reported, as a measured
fact, that the agent had no behaviours loaded. On every machine, since it was
written. Reading that one led here, and here is the rest of them.

WHAT COUNTS AS OURS. A member name in snake_case. It is our convention and it
is the freedesktop convention's opposite: BlueZ, NetworkManager, logind,
StatusNotifierItem and MPRIS all name members in PascalCase (`GetDevices`,
`StartDiscovery`, `SetBrightness`). So the case of the name separates a call
into our own daemons from a call into a service we do not own and cannot check
- 19 of the 36 member calls in the tree are foreign, and every one is
PascalCase. A foreign service with a snake_case member would be read as ours
and land in the list below with a reason; none exists today.

WHAT THIS CANNOT SEE. A call whose member is a variable rather than a literal.
`agent_call_string(member, ...)` takes the name as an argument, and the dead
`list_skills` reached its proxy that way - so this check would have missed the
very defect it was written from, if the name were not also a literal at the
call site. Both halves are read: the literal passed to a helper counts, because
the helper's own call is not a literal at all.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

SKIP = {"target", "node_modules", "mkosi.builddir", ".git", ".svelte-kit", "build", "dist"}

# Where a member can be SERVED, and where one can be DIALLED. A daemon is both:
# the undo service dials the signer, the shell dials the undo service.
SERVERS = ("daemons", "ai", "sdk", "contracts")
CALLERS = ("apps", "daemons", "ai")

EXPLICIT_NAME = re.compile(r'#\[zbus\(name\s*=\s*"([^"]+)"\)\]')
INTERFACE = re.compile(r"#\[(?:zbus::)?interface\b")
METHOD = re.compile(r"(?:async\s+)?fn\s+(\w+)\s*\(\s*&")
# `.call("x", ...)`, `.call_method("X", ...)`, `.call::<_, _, String>("x", ...)`,
# and the helper shapes that pass the member ALONG rather than dialling in place:
# `try_call_string(AI_BUS, AI_PATH, "x")`, `agent_call_string("x")`. The bus and
# path arguments are identifiers, never literals - sometimes consts
# (`AI_BUS`), sometimes locals (`bus`, `path`, in the loop that asks both AI
# principals the same question) - so allowing plain identifiers before the first
# string reaches the member without reaching past it. A member given first is a
# literal and matches immediately.
#
# The first cut matched only `.call("` and the stale-entry half of this check
# caught it within the minute: seven members it had just been told about read as
# no longer dialled, because the literal sits in a helper's argument list. The
# docstring had predicted that limitation and I wrote the narrow regex anyway.
# A METHOD call on a proxy, or one of the `*call_string` helpers. Not a bare
# `call(...)`: code-indexer's tests define a local helper by that exact name and
# eight of its fixtures - `foo`, `bar`, `nope` - read as dialled members.
CALL = re.compile(
    r'(?:\.call(?:_method)?|\b\w*call_string)(?:::<[^>]*>)?\(\s*'
    r'(?:[A-Za-z_][A-Za-z0-9_]*\s*,\s*)*"([^"]+)"'
)
OURS = re.compile(r"^[a-z][a-z0-9_]*$")

# A dialled member with no server, kept with the reason it is still here. The
# list may SHRINK and may not grow: a new one is a fresh surface wired to
# nothing, which is the whole point of the check.
ACKNOWLEDGED: dict[str, str] = {
    "list_skills": "the AI behaviours panel; its shape must be settled first (the "
    "command described an array, the page declares {behaviours, errors}). The "
    "Settings command now fails rather than substituting `[]`, so the panel says it "
    "could not read them instead of reporting none.",
    "ai_models_list": "the model picker's catalogue. Reported with the rest of the "
    "AI management surface; the engine serves no model administration at all.",
    "ai_providers_list": "as ai_models_list.",
    "ai_defaults_get": "as ai_models_list.",
    "ai_set_active": "the model picker's live swap. A write, so pressing it does "
    "nothing today; the picker reports the refusal rather than hiding it.",
    "ai_provider_set_enabled": "as ai_set_active, from both Settings and the harness.",
    "ai_provider_test": "as ai_set_active, from both Settings and the harness.",
    "ai_set_action_mode": "as ai_set_active.",
    "ai_set_autonomous_app": "as ai_set_active.",
    "pending_proposals": "the gate feed. The harness reads it through "
    "`try_call_string`, so an absent member is already reported as unread rather "
    "than as an empty queue.",
    "action_state": "as pending_proposals.",
    "access_grants": "the transparency drawer's grants feed; same handling.",
    "approve": "the gate's decision path, paired with pending_proposals.",
    "deny": "as approve.",
}


def sources(base: Path):
    for p in base.rglob("*.rs"):
        if not set(p.parts) & SKIP:
            yield p


def served(root: Path) -> set[str]:
    """Every member name our own interfaces answer to."""
    names: set[str] = set()
    for name in SERVERS:
        base = root / name
        if not base.is_dir():
            continue
        for p in sources(base):
            text = p.read_text(encoding="utf-8", errors="replace")
            if not INTERFACE.search(text):
                continue
            names.update(EXPLICIT_NAME.findall(text))
            # zbus names an un-renamed method in PascalCase; both spellings are
            # recorded because a caller may legitimately use either form.
            for fn in METHOD.findall(text):
                names.add(fn)
                names.add("".join(w.capitalize() for w in fn.split("_")))
    return names


def called(root: Path) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for name in CALLERS:
        base = root / name
        if not base.is_dir():
            continue
        for p in sources(base):
            text = p.read_text(encoding="utf-8", errors="replace")
            for member in CALL.findall(text):
                out.setdefault(member, []).append(str(p.relative_to(root)))
    return out


def main() -> int:
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT
    have = served(root)
    dial = called(root)
    if not dial:
        print(
            f"NOTHING WAS READ: no D-Bus member calls found under {root}",
            file=sys.stderr,
        )
        return 2

    ours = {m: w for m, w in dial.items() if OURS.match(m)}
    findings: list[str] = []
    known: list[str] = []
    for member, where in sorted(ours.items()):
        if member in have:
            continue
        first = sorted(set(where))[0]
        text = (
            f"{first}: dials `{member}`, and no interface in this tree serves it. "
            f"The call fails every time it is made."
        )
        if member in ACKNOWLEDGED:
            known.append(f"{text}\n      reason: {ACKNOWLEDGED[member]}")
        else:
            findings.append(text)

    # Only against the real tree. The list below describes THIS repo, so in a
    # fixture tree every entry reads as stale and the controls could never test
    # anything else.
    stale = sorted(set(ACKNOWLEDGED) - set(ours)) if root == ROOT else []
    for member in stale:
        findings.append(
            f"`{member}` is acknowledged as dialled-but-unserved and nothing dials it "
            f"any more. Drop the entry."
        )

    print(
        f"{len(ours)} member(s) dialled on our own buses, "
        f"{len(dial) - len(ours)} on services we do not own and cannot check; "
        f"{len(known)} unserved and acknowledged"
    )
    if known:
        print("\ndialled but served by nothing, each with its reason:\n")
        for k in known:
            print(f"  - {k}")
    if findings:
        print("\na surface wired to a member nobody implements:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
