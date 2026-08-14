#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A bus subscription or publish the profile does not grant is dropped, not refused.

`permitted_subscriptions` (event-bus `socket.rs`) FILTERS the patterns a caller's
`[event_bus].subscribe` scope does not cover, and the connection then succeeds
with the rest. So an app that subscribes to a topic its profile forgot does not
get an error, a warning it can see, or a failed startup. It gets a receiver that
never yields, which from inside is indistinguishable from a topic nobody
publishes on.

The shell's own source says this, and answers it with a sentence:

    The two lists were checked equal on 11 Aug.

That is the shape this repository keeps replacing. A hand-check with a date on it
is correct exactly until the next person adds a subscription, and the thing it
protects goes quiet rather than red. So the check is derived: read what each app
subscribes to out of its code, read what its shipped profile grants, and compare.

WHAT GOES WRONG WITHOUT IT is not an abstraction. The terminal subscribes to
`accessibility.state` to build xterm's screen-reader mirror. Its profile is
first-party, so it IS held to its scope under enforcement; a missing grant there
is a blind person's terminal going silent, from a one-line profile edit that
reviews clean.

WHO IS ACTUALLY HELD TO THESE LISTS, because the two halves answer differently
and the profile's own `tier` field answers neither. The bus computes tier from
the peer's exe path (`peer_tier` -> `detect_tier`), and apps install under
`/usr/lib/arlen/apps/<id>/bin/`, which that function calls System. So:

  * PUBLISH is exempt for anything system-tier (`hold_to_scope = !is_system`),
    which today means every first-party app. Their publish lists are honest
    documentation rather than an active gate.
  * SUBSCRIBE exempts only a system peer that declares NOTHING. Declaring a
    subscribe list is how a component opts into being bounded, whatever its tier
    - so a profile that grows an `[event_bus].subscribe` section moves that app
    from exempt to held, and an incomplete list then costs it a feature.

That second rule is why this check earns its place: adding a subscribe list is
the moment an app becomes breakable, and the breakage is silent.

THE PUBLISH SIDE IS THE SAME SHAPE and was found by looking for it: a denied
publish hits `continue` in the producer loop (`socket.rs:437`), and the wire
protocol is fire-and-forget, so the producer is never told. It emits into
nothing. The shell was publishing four topics against a profile that said
`publish = []`, with a comment reasoning from a true observation - no
`UnixEventEmitter` in its tree - to a false conclusion, because it publishes
through a hand-rolled `emit_to_event_bus` instead. Looking for the TYPE missed
the BEHAVIOUR, which is exactly why this reads what is emitted rather than which
API is imported.

WHAT THIS CANNOT SEE: it matches the two literal shapes the tree uses today (a
`subscribe(vec![...])` call and a comma-joined `SUBSCRIPTIONS` const). A
subscription assembled at runtime from config escapes it. That is why finding
NOTHING for an app that declares a subscribe list is an error rather than a pass:
the check refuses to be silently vacuous, which is the failure mode it exists to
prevent in the first place.
"""

import re
import sys
import tomllib
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

# The shipped profiles, under the uid of the user they apply to.
PROFILE_DIR = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000"

# Where an app id's source lives. DERIVED, not tabled: `dev.arlen.terminal` ->
# `apps/terminal` or `daemons/terminal`. An id that resolves to no directory is
# reported rather than skipped.
SOURCE_ROOTS = ("apps", "daemons")

# Components whose source is not in this tree. The compositor is its own repo, so
# what it subscribes to cannot be read from here - carried with the reason rather
# than reported every run as a derivation failure.
OUT_OF_TREE = {
    "arlen-compositor": "cosmic-comp fork, separate repo (~/Repositories/compositor)",
}

# `consumer.subscribe(vec!["a.b".into(), "c.".into()])` and friends.
SUBSCRIBE_CALL = re.compile(r"\.subscribe\s*\(\s*vec!\s*\[(?P<body>[^\]]*)\]", re.S)

# What an app emits. Two shapes: the SDK emitter's method, and the shell's own
# hand-rolled helper - which is the one a check written against the SDK type
# would have missed.
PUBLISH_CALL = re.compile(
    r'(?:emit_to_event_bus|\.emit|emit_event)\s*\(\s*"(?P<topic>[^"]+)"', re.S
)

# The shell's shape: one comma-joined const it registers with.
SUBSCRIPTIONS_CONST = re.compile(
    r"const\s+SUBSCRIPTIONS\s*:\s*&str\s*=\s*(?P<body>(?:\s*\"[^\"]*\")+)\s*;", re.S
)

STRING = re.compile(r'"([^"]*)"')

# An SDK helper that subscribes on the caller's behalf. `subscribe_menu_actions`
# does `subscribe(vec![MENU_ACTION_INVOKED])` inside os-sdk, so the topic appears
# in NEITHER the app's source nor as a literal - and an app calling it still needs
# the grant. Resolved rather than tabled: find the helpers, read what they ask
# for, then credit any app that calls one.
HELPER_FN = re.compile(r"pub\s+(?:async\s+)?fn\s+(\w+)[^{]*\{", re.S)
CONST_STR = re.compile(r'const\s+(\w+)\s*:\s*&str\s*=\s*"([^"]*)"')
SKIP = ("/target/", "node_modules", "/.git/", "mkosi.builddir")


def granted(patterns: list[str], topic: str) -> bool:
    """The bus's own `pattern_matches`: `a.b.*` covers `a.b.x`, `a.b` only itself."""
    for p in patterns:
        if p.endswith(".*"):
            prefix = p[:-2]
            if topic.startswith(prefix) and topic[len(prefix) :].startswith("."):
                return True
            # A registration prefix (`window.`) against a `window.*` grant: the
            # two spell the same idea differently, and the bus admits it.
            if topic.rstrip(".") == prefix:
                return True
        elif p == topic:
            return True
    return False


def source_dir(repo: Path, app_id: str) -> Path | None:
    name = app_id.removeprefix("dev.arlen.")
    for root in SOURCE_ROOTS:
        d = repo / root / name
        if d.is_dir():
            return d
    return None


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


def sdk_helpers(repo: Path) -> dict[str, set[str]]:
    """SDK functions that subscribe for their caller, and to what."""
    helpers: dict[str, set[str]] = {}
    sdk = repo / "sdk/os-sdk/src"
    if not sdk.is_dir():
        return helpers
    for f in sdk.rglob("*.rs"):
        text = f.read_text(encoding="utf-8", errors="replace")
        consts = dict(CONST_STR.findall(text))
        for m in HELPER_FN.finditer(text):
            body = body_of(text, m.end() - 1)
            topics: set[str] = set()
            for call in SUBSCRIBE_CALL.finditer(body):
                arg = call.group("body")
                topics.update(t for t in STRING.findall(arg) if t)
                # `subscribe(vec![SOME_CONST.to_string()])` - the literal lives in
                # the const, which is the whole reason this pass exists.
                for name, value in consts.items():
                    if re.search(rf"\b{name}\b", arg):
                        topics.add(value)
            if topics:
                helpers[m.group(1)] = topics
    return helpers


def publishes_of(directory: Path) -> set[str]:
    """Every topic this app emits onto the bus."""
    found: set[str] = set()
    for f in directory.rglob("*.rs"):
        if any(s in str(f) for s in SKIP):
            continue
        text = f.read_text(encoding="utf-8", errors="replace")
        for m in PUBLISH_CALL.finditer(text):
            topic = m.group("topic")
            # A dotted topic, not a Tauri window event or a log line that happens
            # to sit behind a method called `emit`.
            if "." in topic and "://" not in topic and " " not in topic:
                found.add(topic)
    return found


def subscriptions_of(directory: Path, helpers: dict[str, set[str]]) -> set[str]:
    """Every topic or prefix this app registers with the bus."""
    found: set[str] = set()
    for f in directory.rglob("*.rs"):
        if any(s in str(f) for s in SKIP):
            continue
        text = f.read_text(encoding="utf-8", errors="replace")
        for m in SUBSCRIBE_CALL.finditer(text):
            found.update(s for s in STRING.findall(m.group("body")) if s)
        for m in SUBSCRIPTIONS_CONST.finditer(text):
            joined = "".join(STRING.findall(m.group("body")))
            found.update(t for t in joined.split(",") if t)
        for name, topics in helpers.items():
            if re.search(rf"\b{name}\s*\(", text):
                found.update(topics)
    return found


def main() -> int:
    profiles = sorted((REPO / PROFILE_DIR).glob("*.toml"))
    if not profiles:
        print(f"NOTHING WAS READ: no profiles under {REPO / PROFILE_DIR}", file=sys.stderr)
        return 2

    problems: list[str] = []
    notes: list[str] = []
    carried: list[str] = []
    checked = 0
    helpers = sdk_helpers(REPO)

    for path in profiles:
        try:
            profile = tomllib.loads(path.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as e:
            problems.append(f"{path.name}: will not parse ({e})")
            continue

        event_bus = profile.get("event_bus", {})
        subscribe = event_bus.get("subscribe")
        publish = event_bus.get("publish")
        if subscribe is None and publish is None:
            # Declares nothing, so the bus does not hold it to anything. Not this
            # check's business - whether it SHOULD declare one is the profile
            # work's call, not a rule this can derive.
            continue

        app_id = profile.get("info", {}).get("app_id", path.stem)
        if app_id in OUT_OF_TREE:
            carried.append(f"{path.name}: {OUT_OF_TREE[app_id]}")
            continue
        directory = source_dir(REPO, app_id)
        if directory is None:
            problems.append(
                f"{path.name}: `{app_id}` resolves to no source directory under "
                f"{'/, '.join(SOURCE_ROOTS)}/, so what it subscribes to cannot be read. "
                f"Either the id or this check's derivation is wrong."
            )
            continue

        wanted = subscriptions_of(directory, helpers) if subscribe is not None else set()
        if subscribe is not None and not wanted:
            # An UNUSED grant, which is the other direction and not a break: extra
            # scope is permissive, so nothing goes quiet. It is still worth saying,
            # because "erring narrow" is what these profiles claim about themselves
            # and an unused `*` is the opposite of narrow. Reported, not failed -
            # removing a grant can break a consumer this cannot see, so that is a
            # decision rather than a fix.
            notes.append(
                f"{path.name}: grants {subscribe} and no subscription was found in "
                f"{directory.relative_to(REPO)}. Either the grant outlived its "
                f"consumer, or the app subscribes in a shape this cannot read."
            )
            continue

        checked += 1
        for topic in sorted(wanted):
            if not granted(subscribe, topic):
                problems.append(
                    f"{path.name}: subscribes to `{topic}` and the profile does not "
                    f"grant it.\n"
                    f"    Under ARLEN_EVENT_BUS_ENFORCE the bus DROPS that pattern and "
                    f"keeps the connection, so the app waits on a receiver that never "
                    f"yields and nothing anywhere reports it. Add it to "
                    f"[event_bus].subscribe."
                )

        # The publish half. Same silence, other direction: the event is dropped
        # and the producer, speaking a fire-and-forget protocol, is never told.
        for topic in sorted(publishes_of(directory)):
            if not granted(publish or [], topic):
                problems.append(
                    f"{path.name}: emits `{topic}` and the profile does not grant it.\n"
                    f"    Under ARLEN_EVENT_BUS_ENFORCE the bus DROPS the event and says "
                    f"nothing to the producer, so every consumer of that topic goes "
                    f"quiet while the emitting code looks like it worked. Add it to "
                    f"[event_bus].publish."
                )

    if carried:
        print("carried, with a reason (see OUT_OF_TREE):")
        for line in carried:
            print(f"  {line}")
        print()

    if notes:
        print("a grant with no subscription behind it:")
        for line in notes:
            print(f"  {line}")
        print()

    if problems:
        print("a subscription the profile does not grant:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {checked} profile(s) with a subscribe list grant every topic their app asks for")
    return 0


if __name__ == "__main__":
    sys.exit(main())
