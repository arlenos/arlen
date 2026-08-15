#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Every socket a shipped component dials is served by something shipped.

The mechanism-versus-assembly gap one layer above `check-runtime-deps`: that asks
whether the binaries our code EXECS are in the image, this asks whether the daemons
our code DIALS are. Both failures look identical from outside - `No such file or
directory` inside a feature nobody exercised on the appliance.

Measured on 13 Aug: thirteen daemons carry a written systemd unit that no build
phase installs, `modulesd` is referenced from sixteen crates, and the desktop shell
dialled its socket on every boot forever while nothing answered.

WHY THERE IS A TABLE, when the dialling side needs none. A first version tried to
derive both sides by grep and reported most of the tree as unserved - including
sockets I had just watched bind. The server never mentions the socket's name: both
sides go through a shared resolver, so `daemon.rs` binds `socket_path(...)` while the
literal `"knowledge.sock"` lives in `sdk/os-sdk`. Proximity to a bind finds test
fixtures, which is the only place a name and a bind sit together.

So the SERVER is a hand-kept fact, exactly like the cgroup unit table: we choose the
value, the check refuses the drift. That is not a curated policy list - it is a fact
about the tree, and every entry below was taken either from a boot's own
`listening socket=` line or from reading the binding code.

WHAT IS DERIVED, and therefore not written down anywhere: which components ship (the
install destinations of the image build phases, literal even where the source path is
a variable), and which components dial (any non-test reference to the name).

Three checks, and the middle one is what keeps the table honest:

    unserved   a socket referenced outside tests with no table entry
    stale      a table entry whose socket no longer appears in the tree
    unshipped  a socket whose SERVER is not installed by any build phase, while
               something that IS installed dials it
"""

import pathlib
import re
import subprocess
import sys
from pathlib import Path

# The repo to read. An explicit argument exists so the control can run a MODIFIED
# copy of this file against the real tree - without it a copy in a temp dir resolves
# its root from its own path and reads nothing, which is a green run that checked
# nothing at all.
REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
BUILD_PHASES = REPO / "dev/mkosi/mkosi.build.d"

# socket file name -> the binary that BINDS it.
#
# Sourced from the boot of 13 Aug (`listening socket=` in the guest journal) where a
# daemon runs on the image, and from the binding code otherwise. A value here is a
# claim that THIS binary calls bind on THIS socket.
SERVERS = {
    "ai-engine.sock": "arlen-ai-engine-daemon",
    "ai-engine-drive.sock": "arlen-ai-engine-daemon",
    # Bound by the engine daemon for its sandboxed sidecar: the fixed in-sandbox
    # path that is pi's only egress (`sidecar.rs`), not by `arlen-ai-proxy`, which
    # is a D-Bus service and binds no socket.
    "ai-proxy.sock": "arlen-ai-engine-daemon",
    "audit-ingest.sock": "arlen-auditd",
    "audit-read.sock": "arlen-auditd",
    "capsule.sock": "arlen-capsuled",
    "capsule-control.sock": "arlen-capsuled",
    "clipboard.sock": "arlen-desktop-shell",
    "config-broker.sock": "arlen-config-broker",
    "config-broker-identity.sock": "arlen-config-broker",
    "consent-intake.sock": "arlen-consent-broker",
    "consent-control.sock": "arlen-consent-broker",
    "event-bus-producer.sock": "event-bus",
    "event-bus-consumer.sock": "event-bus",
    "intents.sock": "arlen-desktop-shell",
    "knowledge.sock": "arlen-graph-daemon",
    "launch.sock": "arlen-desktop-shell",
    "notification.sock": "arlen-notifyd",
    "power.sock": "arlen-powerd",
    "search.sock": "arlen-desktop-shell",
    "store.sock": "arlen-store-backend",
    "topbar.sock": "arlen-desktop-shell",
    "undo-signer.sock": "arlen-ai-undo-signer",
    # The terminal APP binds this and the MCP server dials it, which is the reverse
    # of the usual direction and is called out in `arlen-run/src/spawn.rs`.
    "terminal-read.sock": "arlen-terminal",
    "rcd.sock": "arlen-accountsd",
    "transfer.sock": "arlen-transferd",
    "portal-picker.sock": "xdg-desktop-portal-arlen",
    "modulesd.sock": "arlen-modulesd",
    "settings-broker.sock": "arlen-settings-broker",
}

# Violations carried with a reason, because resolving them is a scope call rather
# than a measurement. An entry here is a question for the planner, not a permission
# to leave it: the rule says ship the server or remove the caller, and this records
# which one is being asked for.
# Empty since 15 Aug, when `modulesd` shipped and settled the one entry this
# table has ever held. Kept rather than deleted because the next socket in this
# position needs somewhere to be a question instead of a silence.
#
# The entry claimed the runtime was incomplete - "Tier 1 WASM execution is
# configured but the linker is never populated" - and that had stopped being true.
# `tier1.rs:128` calls `populate_linker`, which registers the real WIT host traits
# for graph, network, events and log; 140 unit tests pass.
#
# One thing to know before reading its test suite: `cargo test --release` fails
# both socket round-trips with `Connection reset by peer`, and that is the
# admission gate working rather than a defect. The tests admit themselves through
# `ARLEN_MODULESD_EXTRA_ADMIT`, which is `#[cfg(debug_assertions)]` on purpose so a
# release binary carries no test affordance at all. Run them in debug.
KNOWN: dict[str, str] = {}

# Sockets served by something outside this tree. Short on purpose: an entry is a
# promise that a package we depend on answers.
FOREIGN = {
    "dbus-session.sock": "dbus-daemon, started by the integration harness",
}


def shipped_binaries():
    """Binary names the image build installs.

    TWO sources, and missing the second is a mistake this check made about itself
    first: the chroot phases install most daemons, but `build-image.sh` cross-builds
    some on the host and stages them straight into `mkosi.extra` from a
    `crate:bin:dest` list. Reading only the phases called `event-bus` unshipped -
    a daemon I had watched serve two sockets an hour earlier.
    """
    names = set()
    for phase in sorted(BUILD_PHASES.glob("*.chroot")):
        for dest in re.findall(r'"\$DESTDIR(/[^"]*)"', phase.read_text(encoding="utf-8")):
            if "/bin/" in dest or "/libexec/" in dest:
                names.add(dest.rsplit("/", 1)[-1])
    builder = REPO / "dev/mkosi/build-image.sh"
    if builder.is_file():
        for line in builder.read_text(encoding="utf-8").splitlines():
            m = re.match(r"^[\w/-]+:([\w.-]+):(/[\w/.-]+)$", line.strip())
            if m and ("/bin/" in m.group(2) or "/libexec/" in m.group(2)):
                names.add(m.group(2).rsplit("/", 1)[-1])
    # Anything already staged in the extra tree ships by definition.
    for sub in ("usr/bin", "usr/lib/arlen/libexec"):
        d = REPO / "dev/mkosi/mkosi.extra" / sub
        if d.is_dir():
            names.update(p.name for p in d.iterdir() if p.is_file())
    return names


def referenced():
    """socket name -> {crate dirs referencing it}, excluding test code.

    A reference below a file's `#[cfg(test)]` is a fixture, not a dependency - the
    tree has fifty-odd `*-test.sock` names that would otherwise drown the real ones.
    """
    grep = subprocess.run(
        # The pattern must allow a directory in the literal too, or it filters the
        # line out before the per-line regex below ever sees it - which is how the
        # same blind spot survived being fixed in one place.
        ["git", "grep", "-n", "-E", r'"[a-zA-Z0-9._/{}-]*[a-z0-9]\.sock"', "--", "*.rs"],
        cwd=REPO, capture_output=True, text=True,
    )
    cache, out = {}, {}
    for hit in grep.stdout.splitlines():
        try:
            rel, lineno, _ = hit.split(":", 2)
        except ValueError:
            continue
        if "/target/" in rel or "mkosi.builddir" in rel or "/tests/" in rel:
            continue
        path = REPO / rel
        if path not in cache:
            body = path.read_text(encoding="utf-8", errors="replace").splitlines()
            cut = next((i for i, l in enumerate(body) if "#[cfg(test)]" in l), len(body))
            cache[path] = (body, cut)
        body, cut = cache[path]
        idx = int(lineno) - 1
        if idx >= cut:
            continue
        # The literal may carry a directory ("arlen/capsule.sock"), so match the
        # tail rather than the whole string - requiring a bare name missed four
        # real servers and called their table entries stale.
        # `{uid}` and friends: the shell builds its path with `format!`, so a
        # pattern without interpolation missed the very caller this check was
        # written for. Third time the same blind spot bit - each place that
        # narrows the character class has to allow the same things.
        for name in re.findall(r'"[a-zA-Z0-9._/{}-]*?([a-z0-9][a-z0-9._-]*\.sock)"', body[idx]):
            out.setdefault(name, set()).add(rel.split("/src/")[0])
    return out


def component_of(crate):
    """The shipped-or-not unit a crate belongs to: `apps/x` or `daemons/x`.

    A dialer is often a LIBRARY compiled into a shipped binary rather than a binary
    itself - `apps/desktop-shell/core` holds the modulesd client and produces no
    binary at all, so a per-crate check called the shell's own caller unshipped and
    reported OK on the one violation this was written for. Grouping by component
    fixes that without a dependency graph.
    """
    parts = pathlib.PurePath(crate).parts
    return "/".join(parts[:2]) if len(parts) >= 2 else crate


def component_ships(crate, ships):
    """Whether anything in this crate's component produces a shipped binary."""
    root = REPO / component_of(crate)
    if not root.is_dir():
        return False
    for manifest in root.rglob("Cargo.toml"):
        if "/target/" in str(manifest):
            continue
        names = set(re.findall(r'^\s*name\s*=\s*"([^"]+)"',
                               manifest.read_text(encoding="utf-8"), re.M))
        if names & ships:
            return True
    return False


def main():
    ships = shipped_binaries()
    if not ships:
        print("NOTHING WAS READ: no install destinations in the build phases", file=sys.stderr)
        return 2

    refs = referenced()
    problems = []

    for name, crates in sorted(refs.items()):
        if name in FOREIGN or name in SERVERS:
            continue
        problems.append(
            f"{name}: referenced by {', '.join(sorted(crates))} and served by no table entry.\n"
            f"    Add it with the binary that binds it - from the daemon's "
            f"`listening socket=` line on a boot, or from the code that calls bind."
        )

    for name in sorted(SERVERS):
        if name not in refs:
            problems.append(
                f"{name}: in the table, referenced nowhere outside tests. Either the "
                f"socket is gone and the entry should go with it, or it moved behind "
                f"a name this check cannot see."
            )

    unshipped, carried = [], []
    for name, server in sorted(SERVERS.items()):
        if server in ships or name not in refs:
            continue
        if name in KNOWN:
            carried.append(f"{name} (served by {server}): {KNOWN[name]}")
            continue
        dialers = sorted(c for c in refs[name] if component_ships(c, ships))
        if dialers:
            unshipped.append((name, server, dialers))

    if unshipped:
        print("a shipped component dials a socket whose server is not installed:")
        for name, server, dialers in unshipped:
            print(f"  {name}: served by {server}, which no build phase installs")
            print(f"    dialled by {', '.join(dialers)}")
        print("  Ship the server, or remove the caller.")
        problems.append(f"{len(unshipped)} socket(s) dialled with no shipped server")

    for name in sorted(KNOWN):
        if name not in SERVERS:
            problems.append(
                f"{name}: carried as a known violation, but no table entry serves it. "
                f"Either it was resolved and this entry should go, or the table lost "
                f"the socket it was about."
            )
        elif SERVERS[name] in ships:
            problems.append(
                f"{name}: carried as unshipped, but {SERVERS[name]} IS installed now. "
                f"Drop the entry - a carried violation that resolved itself reads as "
                f"coverage."
            )

    if carried:
        print("carried, with a reason (see KNOWN):")
        for line in carried:
            print(f"  {line}")
        print()

    if problems:
        print()
        print("socket servers: the table and the tree disagree")
        for p in problems:
            print(f"  {p}")
        return 1

    print(f"OK: {len(SERVERS)} socket(s) served, every dialled one by a shipped binary")
    return 0


if __name__ == "__main__":
    sys.exit(main())
