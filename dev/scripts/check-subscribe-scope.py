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

THREE WAYS A TOPIC GETS SUBSCRIBED, and only the first is visible in the app:
its own code, an SDK helper it calls by name, and a Tauri plugin it merely LINKS.
The last was added on 14 Aug after an enforce boot named two patterns this check
had reported as an unused grant - `tauri-plugin-shell` subscribes them from a
private function that Tauri's plugin init calls, so no amount of reading the
app's source finds them. Three profiles were wrong at once.

THE PUBLISH SIDE HAS THE SAME BLIND SPOT AND NO STATIC FIX, which is worth
stating rather than leaving to be rediscovered. The first enforce boot of the
publish half (14 Aug) denied three topics no scan here could have found: one
COMPOSED at runtime (`format!("audit.ai.{}", ...)`, a string that does not exist
in the source), one built as a STRUCT LITERAL rather than passed to an emit call
(`r#type: "permission.changed"`), and one emitted by an SDK helper a plugin holds
and the FRONTEND drives through a Tauri command, so there is no Rust call site at
all.

I tried the symmetric plugin pass for publishes and threw it away: crediting an
app with every topic its plugin's helpers CAN emit demanded grants for things
apps never send, and a profile inflated with unused grants stops meaning
"least privilege". The subscribe side has a signal that separates involuntary
from optional (a private fn); the publish side does not. So a boot with enforce
on is the oracle for publishes, and this check is the cheap first pass.

WHAT THIS STILL CANNOT SEE: it matches the two literal shapes the tree uses today
(a `subscribe(vec![...])` call and a comma-joined `SUBSCRIPTIONS` const). A
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
# EVERY uid directory, not just 1000. The bus looks a profile up under the
# PEER's uid, so a root daemon's profile lives under 0 - and globbing one uid
# meant the first such profile (the knowledge daemon's) was invisible to this
# check while being perfectly live at runtime.
PROFILE_DIR = "dev/mkosi/mkosi.extra/var/lib/arlen/permissions"

# Where an app id's source lives. DERIVED, not tabled: `dev.arlen.terminal` ->
# `apps/terminal` or `daemons/terminal`. An id that resolves to no directory is
# reported rather than skipped.
SOURCE_ROOTS = ("apps", "daemons", "dev")

# Ids whose source directory is not the id. A small hand table on purpose, with
# the same discipline as OUT_OF_TREE: an id that resolves nowhere is REPORTED,
# so an entry missing here is loud rather than silent, and the table cannot rot
# into skipping something.
DIR_ALIASES = {
    "auditd": "daemons/audit-daemon",
    "knowledge": "daemons/knowledge",
    "notifyd": "daemons/notification-daemon",
    "anomalyd": "daemons/anomaly-detector",
    "clockd": "daemons/clock",
    "powerd": "daemons/power-daemon",
}

# Components whose source is not in this tree. The compositor is its own repo, so
# what it subscribes to cannot be read from here - carried with the reason rather
# than reported every run as a derivation failure.
OUT_OF_TREE = {
    # Keyed on the id the BUS resolves (`/usr/bin/arlen-compositor` -> rule (2)
    # -> `compositor`), not on the profile filename. The two differed until
    # 14 Aug, which is how a profile nothing could load sat here being carried.
    "compositor": "cosmic-comp fork, separate repo (~/Repositories/compositor)",
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

# Any function with its visibility, so a plugin's PRIVATE subscribes can be told
# from a library's public ones. See `plugin_subscriptions`.
ANY_FN = re.compile(r"(?P<vis>pub\s+)?(?:async\s+)?fn\s+\w+[^{;]*\{", re.S)
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
    if app_id in DIR_ALIASES:
        d = repo / DIR_ALIASES[app_id]
        return d if d.is_dir() else None
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


def plugin_subscriptions(repo: Path) -> dict[str, set[str]]:
    """Crate name -> topics it subscribes for any app that merely LINKS it.

    The named-helper pass below needs the app to call the function, which is the
    normal SDK shape. A Tauri plugin is not that shape: `tauri-plugin-shell`
    subscribes `app.toolbar.action_invoked` and `app.shortcut.action_invoked`
    inside `spawn_action_invoked_consumer`, which nothing in the app names -
    Tauri's plugin init calls it. So the trigger is the DEPENDENCY, not a call.

    Missing that cost three profiles at once (14 Aug). An enforce boot named the
    knowledge app's two filtered patterns out loud, and this check had reported
    the opposite: that the grant had no subscription behind it. Both readings
    came from the same blind spot, which the note below already predicted -
    "or made by a helper this cannot follow, looks like no subscription".

    TWO THINGS NARROW IT, because the first cut was wrong in a way worth keeping
    written down. Scanning every `sdk/*` crate for any `.subscribe(vec![...])`
    credited `*` and `app.annotation.` to all three apps, because os-sdk is a
    LIBRARY: its subscribe calls are ones a caller asks for, not ones the app
    inherits by linking. So:

      * only `tauri-plugin-*` crates, which are the ones Tauri initialises itself;
      * only subscribes inside a NON-`pub` function, since a private function is
        one the app cannot have chosen to call.

    `spawn_action_invoked_consumer` is private and reached from plugin init,
    which is exactly the shape that makes a subscription involuntary.
    """
    out: dict[str, set[str]] = {}
    sdk = repo / "sdk"
    if not sdk.is_dir():
        return out
    for cargo in sdk.glob("tauri-plugin-*/Cargo.toml"):
        src = cargo.parent / "src"
        if not src.is_dir():
            continue
        topics: set[str] = set()
        for f in src.rglob("*.rs"):
            text = f.read_text(encoding="utf-8", errors="replace")
            for m in ANY_FN.finditer(text):
                if m.group("vis"):
                    continue
                for call in SUBSCRIBE_CALL.finditer(body_of(text, m.end() - 1)):
                    topics.update(t for t in STRING.findall(call.group("body")) if t)
        if topics:
            out[cargo.parent.name] = topics
    return out


def linked_plugin_topics(directory: Path, plugins: dict[str, set[str]]) -> set[str]:
    """Topics an app inherits from the plugin crates its manifests depend on.

    MATCHED ON THE PATH, not the dependency key, because those differ: the crate
    lives in `sdk/tauri-plugin-shell` and every app depends on it as
    `tauri-plugin-arlen-shell = { path = "../../../sdk/tauri-plugin-shell" }`.
    Keying on the name found nothing at all and the check stayed quietly green,
    which is the failure mode it was written to avoid.
    """
    found: set[str] = set()
    for cargo in directory.rglob("Cargo.toml"):
        if any(s in str(cargo) for s in SKIP):
            continue
        text = cargo.read_text(encoding="utf-8", errors="replace")
        for crate, topics in plugins.items():
            if re.search(rf'path\s*=\s*"[^"]*/sdk/{re.escape(crate)}"', text):
                found.update(topics)
    return found


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


def subscriptions_of(
    directory: Path, helpers: dict[str, set[str]], plugins: dict[str, set[str]]
) -> set[str]:
    """Every topic or prefix this app registers with the bus.

    Three ways it can get there: its own code, an SDK helper it calls by name,
    and a plugin crate it merely links. The last one is invisible in the app's
    source, which is exactly why it has to be read from the manifest.
    """
    found: set[str] = set(linked_plugin_topics(directory, plugins))
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
    profiles = sorted((REPO / PROFILE_DIR).glob("*/*.toml"))
    if not profiles:
        print(f"NOTHING WAS READ: no profiles under {REPO / PROFILE_DIR}", file=sys.stderr)
        return 2

    problems: list[str] = []
    notes: list[str] = []
    carried: list[str] = []
    checked = 0
    helpers = sdk_helpers(REPO)
    plugins = plugin_subscriptions(REPO)

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

        wanted = (
            subscriptions_of(directory, helpers, plugins)
            if subscribe is not None
            else set()
        )
        # An EMPTY list granting nothing to an app that subscribes to nothing is
        # the correct end state, not a finding - it is how a profile says "hears
        # nothing" while staying declared, which is what keeps a system-tier app
        # bounded instead of exempt. Only a non-empty grant with nothing behind
        # it is worth a word.
        if subscribe and not wanted:
            # An UNUSED grant, which is the other direction and not a break: extra
            # scope is permissive, so nothing goes quiet. It is still worth saying,
            # because "erring narrow" is what these profiles claim about themselves
            # and an unused `*` is the opposite of narrow.
            #
            # REPORTED, NEVER FAILED, and the sibling gate explains why better than
            # this one could: `check-read-grants-cover-queries.py` refuses to report
            # unused grants at all, because "a wrongly-reported unused grant invites
            # deleting a grant that IS needed", and its author nearly made that
            # mistake by hand on a multi-line query a regex had not seen. The same
            # blindness applies here - a subscription assembled at runtime, or made
            # by a helper this cannot follow, looks like no subscription.
            #
            # So the note names BOTH causes and commits to neither, and the repair
            # is a boot with enforce on, where a real consumer announces itself as a
            # denial line. Measure it; do not delete on the strength of a scan.
            notes.append(
                f"{path.name}: grants subscribe {subscribe} and no subscription was "
                f"found in {directory.relative_to(REPO)}. Either the grant outlived "
                f"its consumer, or the app subscribes in a shape this cannot read."
            )
            # NO `continue`: the publish half below is a separate question about
            # the same file, and skipping it here meant a profile with both
            # problems only ever reported one. Found by running the check after
            # adding the publish note and not seeing it fire.

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
        emitted = publishes_of(directory)
        if publish and not emitted:  # non-empty only, same reason as above
            # The mirror of the unused-subscribe note. Reporting one and not the
            # other would make the gate quietly one-eyed, and an unused publish
            # grant is the likelier paste: publish and subscribe lists travel
            # together when a profile is copied between components.
            notes.append(
                f"{path.name}: grants publish {publish} and emits nothing from "
                f"{directory.relative_to(REPO)}."
            )
        for topic in sorted(emitted):
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
