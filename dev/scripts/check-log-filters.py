#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""An app's log filter must name its own crate, not set a level for everything.

Two defects came out of one line, in thirteen apps, and neither was anyone's
decision:

    default_filter_or("info")   sets info for every crate in the process, so zbus
                                logs its D-Bus handshake frames WITH the message
                                bytes. A message body is user content - file
                                paths, query strings, notification text - and at
                                info it lands in a journal that no capability
                                grant covers. That is the graph's whole scoping
                                story undone in a log line.
    env_logger::init()          defaults to `error`, so the app is mute: every
                                log::info! and log::warn! in it produces nothing.
                                This is what made the boot consent hang so hard
                                to find - the component in the middle could not
                                be heard - and it was true of four apps at once.

The shape that avoids both is `warn,<own_crate>=info`: the app speaks, its
dependencies do not. So the rule is that the filter must MENTION a crate. A bare
level, in either direction, is the thing that produced both defects.

It does not check WHICH crate or which levels - `warn,x=info` and
`error,x=debug,y=trace` are both somebody's considered choice. It refuses only
the blanket, because a blanket is what nobody chooses and everybody inherits.
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
APPS = ROOT / "apps"

# A log line, in either vocabulary. Used to work out which of a component's
# crates actually speak, which is what decides how many names its filter needs.
LOG_CALL = re.compile(
    r"(?:tracing|log)::(?:trace|debug|info|warn|error)!|"
    r"(?:^|[^\w:])(?:trace|debug|info|warn|error)!\s*\("
)

# The components that carry a bare `EnvFilter::new("info")` today: a level for
# every crate in the process, dependencies included, which is the same defect the
# app half refuses in `env_logger`'s spelling. They are a QUEUE, not an excuse.
#
# They are not swept because the fix is not mechanical. A `tracing` target roots
# at the crate the line was compiled into, so a daemon whose logic sits in a lib
# behind a thin `main.rs` needs BOTH names, and a filter naming one of the two
# makes the other half mute - which is the defect, not the fix. Each wants its
# own sitting with the daemon run afterwards.
#
# The knowledge daemon shows the other shape: it names both `knowledge` and
# `arlen_graph_daemon`, because its bin was renamed and one directive alone had
# stopped matching what it emits.
#
# The picker is the first one done, and it is the shape that makes it easy: its
# `main.rs` logs nothing at all, so naming the lib crate mutes nothing. Check that
# before taking the next one off this list, and take the name off when you do.
#
# The wallpaper daemon is the second, and it is the shape the deferral warns
# about: 13 calls in the binary crate and 5 in the library, so the filter names
# both. `daemons/wallpaper/src/main.rs` carries the test that proves the string
# does what it says - events emitted with explicit targets, captured, asserted -
# and that test is the pattern for the rest of this list. Counting the calls per
# crate first is the whole job; the string follows from it.
TRACING_QUEUE: frozenset[str] = frozenset(
    {
        "ai-proxy",
        "ai-undo-signer",
        "ai-engine-daemon",
        "anomaly-detector",
        "bridge-ingest",
        "calendar",
        "clock",
        "code-indexer",
        "config-broker",
        "file-manager-mcp",
        "knowledge-mcp",
        "notification-daemon",
        "online-accounts",
        "session-supervisor",
        "system-monitor-mcp",
        "terminal-run-mcp",
        "transfer-daemon",
        "xdg-portal",
    }
)
DAEMONS = ROOT / "daemons"

# app -> why its filter is not ours to fix.
NOT_OURS: dict[str, str] = {
    "store": (
        "arlen-ui's live work; the same blanket `info` is there and the fix is the "
        "same one line, but editing their tree mid-flight is worse than the defect"
    ),
}


def _code_only(text: str) -> str:
    """Drop `//` line comments before matching.

    Without this the check reads its own advice back as a defect: the fix it asks
    for is usually written up in a comment right above the call, quoting the bad
    form to explain why it was replaced. Scanning that as code made all four
    already-fixed apps fail - the same "counted the wrong thing" mistake as the
    fixture guard, one night apart.
    """
    return "\n".join(re.sub(r"//.*$", "", line) for line in text.splitlines())



def _without_tests(text: str) -> str:
    """The text up to its first test module.

    A test that emits `info!(target: "wayland_client", …)` to prove a filter is
    not the crate asking for that target in production, and reading it as one
    turned this file's own regression test into a finding against three unrelated
    components - through a dependency edge, no less.
    """
    cut = text.find("#[cfg(test)]")
    return text if cut < 0 else text[:cut]


def _crate_names(comp: pathlib.Path) -> tuple[set[str], set[str]]:
    """The crate names a component compiles: (library, binaries), underscored.

    A `tracing` target roots at the CRATE a line was compiled into, not at the
    package or the directory, and the three differ often enough in this tree to
    matter: `connections` ships a lib called `connections` and a bin called
    `arlen-connectionsd`. Read from Cargo.toml rather than guessed, and returned
    apart, because the rule below needs to know which side a name belongs to.
    """
    # An app keeps its manifest under `src-tauri/`, a daemon at its root. Looking
    # only at the root returned nothing for all nineteen Tauri frontends, and the
    # rule below then skipped every one of them without saying so - a check that
    # passes because it read nothing, which is the shape this file keeps finding
    # elsewhere.
    manifest = next(
        (m for m in (comp / "Cargo.toml", comp / "src-tauri" / "Cargo.toml") if m.is_file()),
        None,
    )
    if manifest is None:
        return set(), set()
    text = manifest.read_text(encoding="utf-8", errors="replace")
    package = re.search(r'(?m)^\s*name\s*=\s*"([^"]+)"', text)
    lib: set[str] = set()
    bins: set[str] = set()
    section = None
    for line in text.splitlines():
        head = line.strip()
        if head.startswith("["):
            section = head
            continue
        m = re.match(r'\s*name\s*=\s*"([^"]+)"', line)
        if not m:
            continue
        if section == "[lib]":
            lib.add(m.group(1).replace("-", "_"))
        elif section == "[[bin]]":
            bins.add(m.group(1).replace("-", "_"))
    if package:
        # An implicit lib or bin takes the package's name.
        if (comp / "src" / "lib.rs").is_file() and not lib:
            lib.add(package.group(1).replace("-", "_"))
        if (comp / "src-tauri" / "src" / "lib.rs").is_file() and not lib:
            lib.add(package.group(1).replace("-", "_"))
        if not bins:
            bins.add(package.group(1).replace("-", "_"))
    return lib, bins


def _path_dep_crates(comp: pathlib.Path) -> set[str]:
    """Crate names this component pulls in from elsewhere in the tree.

    Naming one is legitimate and often the point: the desktop shell keeps half its
    logic in `apps/desktop-shell/core`, and a filter that named only the shell's
    own crates would leave that half mute. They are ALLOWED, never required - a
    dependency that says nothing needs no directive.
    """
    manifest = next(
        (m for m in (comp / "Cargo.toml", comp / "src-tauri" / "Cargo.toml") if m.is_file()),
        None,
    )
    if manifest is None:
        return set()
    base = manifest.parent
    out: set[str] = set()
    for m in re.finditer(r'(?m)^\s*([A-Za-z0-9_-]+)\s*=\s*\{[^}\n]*path\s*=\s*"([^"]+)"', manifest.read_text(encoding="utf-8", errors="replace")):
        out.add(m.group(1).replace("-", "_"))
        dep = (base / m.group(2)).resolve()
        lib, bins = _crate_names(dep)
        out |= lib | bins
    return out


# A log line that names its own target, and the level it does it at. A target is
# not a crate: `tracing::info!(target: "audit", …)` lands under `audit`, so a
# filter naming only crates leaves it at the default level whatever the crate
# directive says.
EXPLICIT_TARGET = re.compile(
    r'(?:tracing::)?(trace|debug|info)!\s*\(\s*\n?\s*target:\s*"([^"]+)"',
    re.MULTILINE,
)


# What a component has to be doing for a dependency's explicit target to be
# reachable from it. One entry today: the `audit` target belongs to
# `arlen-permissions`' peer authentication, so a component that never calls it
# cannot emit that line and does not need the directive.
TARGET_ENTRYPOINTS: dict[str, tuple[str, ...]] = {
    "audit": ("ConnectionAuth", "StampedIdentity", "stamped_identity"),
}


def _fine_targets(comp: pathlib.Path) -> set[str]:
    """Explicit log targets this component can emit BELOW warn, its in-tree
    dependencies included.

    `arlen-permissions` writes the identity-cutover line as
    `info!(target: "audit", event = "identity.legacy_unavailable", …)`, and its own
    comment says reading that as noise is how the cutover gets talked out of
    hardening. Under `warn,<crate>=info` it is gone: the directive names a crate
    and the line is filed under `audit`. Every daemon that authenticates a peer
    inherits that, which is why this is read from the dependencies too rather than
    left to whoever writes the next filter to remember.
    """
    out: set[str] = set()
    roots = [comp / "src", comp / "src-tauri" / "src"]
    own = "\n".join(
        f.read_text(encoding="utf-8", errors="replace")
        for r in roots
        if r.is_dir()
        for f in sorted(r.rglob("*.rs"))
    )
    manifest = next(
        (m for m in (comp / "Cargo.toml", comp / "src-tauri" / "Cargo.toml") if m.is_file()),
        None,
    )
    # A dependency CONTAINING the line is not the same as this component reaching
    # it. Settings pulls in `arlen-permissions` for the revoke request types and
    # never authenticates a peer, so requiring `audit` there would be a directive
    # for something that can never fire. The entry points below are what makes the
    # difference readable without guessing at reachability.
    if manifest is not None and any(sym in own for syms in TARGET_ENTRYPOINTS.values() for sym in syms):
        base = manifest.parent
        for m in re.finditer(
            r'(?m)^\s*[A-Za-z0-9_-]+\s*=\s*\{[^}\n]*path\s*=\s*"([^"]+)"',
            manifest.read_text(encoding="utf-8", errors="replace"),
        ):
            roots.append((base / m.group(1)).resolve() / "src")
    for root in roots:
        if not root.is_dir():
            continue
        for f in sorted(root.rglob("*.rs")):
            for m in EXPLICIT_TARGET.finditer(
                _without_tests(_code_only(f.read_text(encoding="utf-8", errors="replace")))
            ):
                out.add(m.group(2))
    return out


def _logs_outside_main(comp: pathlib.Path) -> bool:
    """Whether anything but `main.rs` in this component emits a log line."""
    for rel in ("src", "src-tauri/src"):
        root = comp / rel
        if not root.is_dir():
            continue
        for f in sorted(root.rglob("*.rs")):
            if f.name == "main.rs":
                continue
            if LOG_CALL.search(
                _without_tests(_code_only(f.read_text(encoding="utf-8", errors="replace")))
            ):
                return True
    return False


def _logs_in_main(comp: pathlib.Path) -> bool:
    """Whether the binary's own `main.rs` emits a log line."""
    for rel in ("src/main.rs", "src-tauri/src/main.rs"):
        f = comp / rel
        if f.is_file() and LOG_CALL.search(
            _without_tests(_code_only(f.read_text(encoding="utf-8", errors="replace")))
        ):
            return True
    return False


def _components() -> list:
    """Every component that initialises logging, apps AND daemons.

    Daemons were outside this check until 18 August, and the last component in the
    tree still calling a bare `env_logger::init()` was the eBPF sensor - whose
    whole failure mode is silence. Its journal held four systemd lines and nothing
    of its own, so which of its four tracepoints attached could not be read
    anywhere, on any boot. A check that names a defect and cannot see half the
    processes that can carry it is the shape this file exists to argue against.

    NOT ENFORCED HERE, and worth knowing before someone reaches for it: all 24
    daemons use `EnvFilter::new("info")`, a bare level, ten of them alongside zbus.
    That is the same blanket the app half refuses. It is not added to the rule yet
    because the fix is NOT mechanical: a `tracing` target roots at the crate the
    line was compiled into, so a daemon whose logic sits in a lib behind a thin
    `main.rs` needs BOTH names, and a filter naming one of the two makes the other
    half mute - which is the defect, not the fix. It wants a sitting with each
    daemon run afterwards, not a sweep.
    """
    out = []
    for base, rel, *by_subdir in (
        (APPS, ("src-tauri", "src")),
        (DAEMONS, ("src",)),
        # A frontend under `daemons/` keeps its Rust in `src-tauri/src` like an
        # app, not in `src` like a daemon, so it needs its own pass: the picker's
        # logging init was in neither of the two above. It is named after the
        # frontend, not its parent - merged into the parent, the picker inherited
        # the portal daemon's place on the queue and its own blanket was excused
        # by an entry about a different process.
        (DAEMONS, ("src-tauri", "src"), True),
        # The AI workspace is its own tree of daemons - the proxy and the undo
        # signer both initialise logging and were read by nothing here.
        (ROOT / "ai", ("src",)),
    ):
        if not base.is_dir():
            continue
        for comp in sorted(base.iterdir()):
            if not comp.is_dir():
                continue
            if by_subdir:
                # One entry per sub-crate, under its own name.
                for sub in sorted(c for c in comp.iterdir() if c.is_dir()):
                    root = sub.joinpath(*rel)
                    if not root.is_dir():
                        continue
                    files = sorted(root.glob("*.rs"))
                    if files:
                        out.append(
                            (
                                sub.name,
                                "\n".join(_code_only(f.read_text()) for f in files),
                                sub,
                            )
                        )
                continue
            # A daemon may hold its crate one level down (kernel-layer/kernel-layer).
            roots = [comp.joinpath(*rel)]
            roots += [c.joinpath(*rel) for c in sorted(comp.iterdir()) if c.is_dir()]
            files = [f for r in roots if r.is_dir() for f in sorted(r.glob("*.rs"))]
            if not files:
                continue
            out.append(
                (comp.name, "\n".join(_code_only(f.read_text()) for f in files), comp)
            )
    return out


def main() -> int:
    if not APPS.is_dir():
        print(f"NOTHING WAS READ: no apps directory under {ROOT}", file=sys.stderr)
        return 2

    # `apps/*/src-tauri` plus the frontends under `daemons/`. The picker is a
    # Tauri binary with its own subscriber, and its D-Bus frames name the paths a
    # person just browsed - the blanket `info` this refuses is worse there than
    # in an app, not better.
    sources = sorted(APPS.glob("*/src-tauri/src/*.rs"))
    sources += sorted((ROOT / "daemons").glob("*/*/src-tauri/src/*.rs"))
    if not sources:
        print(f"NOTHING WAS READ: no app sources under {APPS}", file=sys.stderr)
        return 2

    problems: list[str] = []
    checked = 0
    for app, text, comp_dir in _components():
        if "env_logger" not in text and "tracing_subscriber" not in text:
            continue
        checked += 1
        if app in NOT_OURS:
            continue

        bare_init = re.search(r"\benv_logger::init\(\)", text)
        blanket = re.search(r'default_filter_or\(\s*"(trace|debug|info|warn|error)"\s*\)', text)
        # The same blanket in `tracing`'s spelling. It was invisible here until
        # 22 August because the regex knew only `env_logger`, and 27 components
        # carry it - see TRACING_QUEUE.
        tracing_blanket = re.search(
            r'EnvFilter::new\(\s*"(trace|debug|info|warn|error)"\s*\)', text
        )
        if bare_init:
            problems.append(
                f"{app}: `env_logger::init()` defaults to `error`, so this app is mute "
                f"in the journal. Use `default_filter_or(\"warn,<its_crate>=info\")`."
            )
        elif tracing_blanket and app not in TRACING_QUEUE:
            problems.append(
                f"{app}: `EnvFilter::new(\"{tracing_blanket.group(1)}\")` sets a level "
                f"for EVERY crate in the process, dependencies included - which is how "
                f"zbus message bytes reached the journal. Name the component's own "
                f"crate: `\"warn,<its_crate>=info\"`."
            )
        elif named := re.search(
            r'(?:default_filter_or|EnvFilter::new)\(\s*"([^"]*=[^"]*)"', text
        ):
            # The filter names crates. Two ways that goes wrong, both silent:
            # a name no crate here has (a typo, or a rename the filter missed),
            # and a component whose second crate speaks and is not named.
            lib, bins = _crate_names(comp_dir)
            # Targets belong here too: a directive may legitimately name one, and
            # the rule below REQUIRES some of them. Left out, the two halves of
            # this check contradicted each other - `audit=info` was demanded by
            # one and reported as a crate that does not exist by the other.
            targets = _fine_targets(comp_dir)
            have = lib | bins | _path_dep_crates(comp_dir) | targets
            directives = {
                d.split("=", 1)[0].strip()
                for d in named.group(1).split(",")
                if "=" in d
            }
            unknown = sorted(d for d in directives if d and d not in have)
            if unknown and have:
                problems.append(
                    f"{app}: the log filter names {', '.join(unknown)}, which is not a "
                    f"crate this component compiles or depends on in-tree "
                    f"({', '.join(sorted(have))}). A "
                    f"directive for a crate that does not exist matches nothing, so "
                    f"those lines never reach the journal."
                )
            elif have:
                missing = []
                if lib and _logs_outside_main(comp_dir) and not (lib & directives):
                    missing.append(f"the library ({', '.join(sorted(lib))})")
                if bins and _logs_in_main(comp_dir) and not (bins & directives):
                    missing.append(f"the binary ({', '.join(sorted(bins))})")
                # Only for a tracing filter. An explicit target is tracing's
                # vocabulary; an `env_logger` filter selects `log` records, and a
                # tracing event only becomes one when the bridge is compiled in.
                # Requiring the directive there would be cargo-cult - and the
                # shell, which is the component it fired on, is env_logger.
                fine = {
                    t
                    for t in targets
                    if t not in directives and "::" not in t
                } if "EnvFilter::new" in named.group(0) else set()
                if fine:
                    problems.append(
                        f"{app}: the log filter does not name {', '.join(sorted(fine))}, "
                        f"which this component or one of its in-tree dependencies logs "
                        f"to below warn. A target is not a crate: the line is filed "
                        f"under that name, so a directive for the crate does not reach "
                        f"it and the level falls back to the blanket."
                    )
                if missing:
                    problems.append(
                        f"{app}: the log filter names {', '.join(sorted(directives))} "
                        f"and leaves {' and '.join(missing)} at the blanket level, "
                        f"though it logs. A tracing target roots at the crate the line "
                        f"was compiled into, so the unnamed half is mute."
                    )
        elif blanket:
            problems.append(
                f"{app}: `default_filter_or(\"{blanket.group(1)}\")` sets a level for "
                f"EVERY crate in the process, dependencies included - which is how "
                f"zbus message bytes reached the journal. Name the app's own crate: "
                f"`\"warn,<its_crate>=info\"`."
            )

    # Only against the real tree. `NOT_OURS` names apps in THIS repo, so checking
    # it against a fixture asks whether a made-up directory contains `store` - and
    # the answer is always no, which failed every fixture the control planted. An
    # excuse list is a claim about one tree; validating it elsewhere is a category
    # error, and the control is what surfaced it.
    # The queue must stay a queue: a component that has since been given a real
    # filter has to come off the list, or the list slowly becomes the answer
    # instead of the backlog.
    if len(sys.argv) <= 1:
        carrying = {
            app
            for app, text, _ in _components()
            if re.search(r'EnvFilter::new\(\s*"(trace|debug|info|warn|error)"\s*\)', text)
        }
        for done in sorted(TRACING_QUEUE - carrying):
            problems.append(
                f"{done} is on the blanket-filter queue and no longer sets one; "
                f"delete the entry"
            )

    if len(sys.argv) <= 1:
        for stale in sorted(NOT_OURS):
            if not (APPS / stale).is_dir():
                problems.append(f"{stale} is excused here and no longer exists; delete the entry")

    if problems:
        print("app log filters:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} component(s) initialise logging; each names its own crate rather than "
        f"setting a level for every dependency ({len(NOT_OURS)} excused)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
