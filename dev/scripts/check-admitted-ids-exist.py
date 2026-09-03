#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A caller allowlist may not name an id nothing on the image can be.

WHAT THIS IS FOR. `installd` shipped with `INSTALL_CALLERS = ["store"]`. Every app
on the image lives at `/usr/lib/arlen/apps/dev.arlen.<name>/`, so the resolver
answers `dev.arlen.<name>` and no path could ever produce the bare id `store`. The
daemon refused EVERY caller, for reading and for installing, and Settings' Remove
button reported the truth about a machine that could not remove anything.

Nothing caught it. The gate that reads these lists checks their SHAPE - no `dev.`
prefix, exact ids - which was satisfied. A unit test asserting `caller_may_mutate
("store")` passed, because it asked the list about the same string the list holds.
And no boot noticed, because the two callers the list named are not on the image
either, so there was nothing there to be refused.

That is the shape worth gating: a gate closed against everyone looks exactly like
a gate that works. It only shows when someone drives the surface, which for an
install daemon means having something installed to remove.

HOW IT DECIDES. It resolves what the image STAGES through the same rules
`path_to_app_id` uses:

  * `/usr/bin/arlen-<name>` is a system daemon and resolves to `<name>` (rule 2);
  * `/usr/lib/arlen/apps/<id>/...` is an app and resolves to `<id>` (rule 3);
  * a handful of canonical paths resolve to a fixed id (rule 1), and those pairs
    are read out of `identity.rs` rather than copied here.

An id no staged path can produce is reported, unless it is acknowledged below as a
caller that is not packaged yet - which is a real and honest state, and different
from a typo in a scheme.

WHAT IT DOES NOT CHECK. Debug lists (`DEV_ADMITTED`, `*_ADMITTED_DEV`) hold
`dev.<binary>` ids for cargo-run builds, which by design no image produces. They
are skipped rather than acknowledged one by one.

The two root helpers under `daemons/installd/` are skipped too, and for a
different reason worth stating: their `ALLOWED_CALLERS` holds BINARY NAMES read
from `/proc/{pid}/exe`, not resolved app ids. That check is the spoofable basename
comparison the F3 work is replacing; measuring it against the resolver's scheme
would report four correct entries as wrong and say nothing about the real problem,
which is that a basename is not an identity.

Run: dev/scripts/check-admitted-ids-exist.py [repo-root]
"""

import pathlib
import re
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

#: Ids an allowlist names that the image does not yet install, with why. Being on
#: this list is a statement that the caller is COMING, not that the entry is
#: fine: each one is a surface that cannot reach its daemon today.
NOT_PACKAGED_YET = {
    "org.arlen.calendar": (
        "shared-entity writer, and the reason is NOT that the app is missing any "
        "more - `apps/calendar` ships and `04k-calendar.sh.chroot` stages it. It "
        "is staged at `/usr/lib/arlen/apps/dev.arlen.calendar/`, so resolver rule "
        "(3) gives it the id `dev.arlen.calendar`, which is neither this string "
        "nor an `org.arlen.` prefix - so nothing that runs on the image can "
        "present it. Building the app was not what this entry was waiting for; "
        "reconciling the two spellings is, and that is the same question the "
        "shell's `dev.arlen.desktop-shell` pair below already answers once"
    ),
    "org.arlen.contacts": "shared-entity writer for an app that does not exist yet",
    "org.arlen.places": "shared-entity writer for an app that does not exist yet",
    "system": "the daemons' own reserved principal, not a binary on disk",
    "desktop-shell": (
        "the OTHER half of a pair the audit daemon admits on purpose. The image stages the "
        "shell under `/usr/lib/arlen/apps/dev.arlen.desktop-shell/bin/` with a convenience "
        "symlink at `/usr/bin/arlen-desktop-shell`, and `dev.arlen.desktop-shell` IS produced "
        "and IS admitted beside this one. Peer auth resolves `/proc/<pid>/exe`, which names "
        "the real file, so the dotted spelling is the one that arrives; the audit daemon "
        "measured that on 21 Aug and added it. This bare form is what rule (2) would give for "
        "the LINK, kept as belt and braces. An id no peer can present cannot be forged into, "
        "so it costs nothing - but it is dead, and if the shell ever stops being staged as an "
        "app this entry is what would be left holding the admission"
    ),
    "ai-daemon": "no image build phase stages the ai-daemon binary and it carries no unit",
    "ai-engine": (
        "not an admission at all: `ENGINE_APP_ID` is the identity actions are ATTRIBUTED to "
        "under an engine session, placed in the capability's `autonomous_apps`, and nothing "
        "peer-authenticates against it. The engine's own binary resolves to `ai-agent`, so the "
        "two strings do differ - but whether they should be one is a question about the "
        "capability model rather than a typo, and it is dormant while `executor_live` is off"
    ),
    "org.arlen.accounts.rclone": (
        "online-accounts is not part of the image scope (15 Aug), and `rclone` - the binary this "
        "id names a launcher for - is not on the image either. Both halves absent, so the id "
        "names a caller that cannot exist yet"
    ),
}

#: Lists whose entries are development ids by construction.
DEV_LIST = re.compile(r"(^|_)DEV(_|$)")

LIST_CONST = re.compile(
    r"const\s+([A-Z0-9_]*(?:ADMITTED|CALLERS|WRITERS|READERS)[A-Z0-9_]*)\s*"
    r":\s*&\[&str\]\s*=\s*&\[(.*?)\];",
    re.S,
)
#: A caller id written as ONE constant rather than a list. Added 20 August, after
#: two live cases in a morning that this check could not see: the clock daemon
#: admitted `clock` while the image produces `dev.arlen.clock`, so the clock app
#: was refused by its own daemon and said "cannot read your saved clock data"; and
#: the shell filed its own launches under `desktop-shell` while a peer resolves it
#: to `dev.arlen.desktop-shell`, so the ledger carried two names for one
#: application. Both were single `&str` constants, so `LIST_CONST` walked past
#: them.
#:
#: Narrow on purpose. Only names ending in the four suffixes below, and only
#: values shaped like an app id - a `&str` const with a plausible name is a much
#: wider net than a list, and a check that reports thirty strings nobody meant as
#: allowlists is one nobody reads. Seven matches tree-wide when this was written,
#: all of them genuinely caller ids.
SINGLE_CONST = re.compile(
    r"const\s+([A-Z0-9_]*(?:_APP|_APP_ID|_CALLER|_CLIENT_ID))\s*:\s*&str\s*=\s*"
    r'"([a-z][a-z0-9.@_-]*)"'
)
#: `"/some/path" => { return Ok("id".to_string()); }` - rule (1) in the resolver.
STRICT_RULE = re.compile(
    r'"(/usr/[^"]+)"\s*=>\s*\{[^}]*?Ok\("([a-z0-9.@_-]+)"', re.S
)
#: Any install path a phase names. Deliberately NOT anchored to `$DESTDIR`: the
#: install-path phase builds its destinations from a `binary:path` pair list and
#: writes `"$DESTDIR$dest"`, so an anchored pattern sees a variable and reports
#: three working daemons as unreachable. Comment lines are dropped first, so a
#: path named in prose does not count as staged.
STAGED = re.compile(r'(/usr/(?:bin|lib)/arlen[A-Za-z0-9._/-]*)')
SKIP_DIRS = {"target", "node_modules", "mkosi.builddir", "mkosi.cache", "mkosi.tools"}
#: Components whose allowlist holds binary names rather than app ids. See the
#: note above: this is the pre-F3 basename check, and holding it to the
#: resolver's scheme measures the wrong thing.
BASENAME_CHECKS = {"install-helper", "permission-helper"}


def strip_comments(text: str) -> str:
    """Drop `//` comments, so a quoted id inside prose is not read as an entry."""
    return "\n".join(re.sub(r"//.*$", "", line) for line in text.splitlines())


def admitted(root: pathlib.Path) -> dict[str, str]:
    """Every id a release allowlist names, mapped to where it is named."""
    out: dict[str, str] = {}
    for area in ("daemons", "ai", "sdk", "store-backend"):
        for p in (root / area).rglob("*.rs"):
            if SKIP_DIRS & set(p.parts) or BASENAME_CHECKS & set(p.parts):
                continue
            text = strip_comments(p.read_text(errors="replace"))
            for name, body in LIST_CONST.findall(text):
                if DEV_LIST.search(name):
                    continue
                for entry in re.findall(r'"([^"]+)"', body):
                    out.setdefault(entry, f"{p.relative_to(root)}:{name}")
            for name, entry in SINGLE_CONST.findall(text):
                if DEV_LIST.search(name):
                    continue
                out.setdefault(entry, f"{p.relative_to(root)}:{name}")
    return out


def strict_rules(root: pathlib.Path) -> dict[str, str]:
    """The canonical path-to-id pairs of resolver rule (1), read from the source.

    A missing resolver is not an empty rule set: it means this is not the tree the
    check was pointed at, and reporting "everything agrees" would be a lie about a
    place it never looked.
    """
    src = root / "sdk/permissions/src/identity.rs"
    if not src.is_file():
        print(f"NOTHING WAS READ: no resolver at {src}", file=sys.stderr)
        raise SystemExit(2)
    return {path: app_id for path, app_id in STRICT_RULE.findall(src.read_text())}


def deferred_daemons(root: pathlib.Path) -> dict[str, str]:
    """Ids belonging to daemons another gate already records as not-yet-shipped.

    `check-shipped-units.py` keeps the list of units deliberately left off the
    image, each with a reason and a date. Those daemons' ids are absent from the
    image for a decided reason, and repeating them here would be a second copy of
    a decision - the kind that goes stale on one side and reads as two facts.

    The mapping is the unit's own `ExecStart`, resolved through the same rules as
    everything else, so a daemon that gets shipped stops being deferred here the
    moment its unit leaves that list.
    """
    gate = root / "dev/scripts/check-shipped-units.py"
    if not gate.is_file():
        print(f"NOTHING WAS READ: no unit gate at {gate}", file=sys.stderr)
        raise SystemExit(2)
    text = gate.read_text()
    block = text[text.index("NOT_YET_DEPLOYED"):]
    block = block[: block.index("\n}")]
    units = set(re.findall(r'"([a-z0-9@.-]+\.service)"', block))
    strict = strict_rules(root)
    out: dict[str, str] = {}
    for unit in root.glob("**/dist/**/*.service"):
        if SKIP_DIRS & set(unit.parts) or unit.name not in units:
            continue
        for m in re.findall(r"^ExecStart=(\S+)", unit.read_text(), re.M):
            if m in strict:
                out[strict[m]] = f"{unit.name} is deliberately not on the image"
            elif m.startswith("/usr/bin/arlen-"):
                out[m[len("/usr/bin/arlen-"):]] = f"{unit.name} is deliberately not on the image"
    return out


def producible(root: pathlib.Path) -> dict[str, str]:
    """Every id the image can resolve, mapped to the path that produces it."""
    strict = strict_rules(root)
    out: dict[str, str] = {}
    phases = list((root / "dev/mkosi/mkosi.build.d").glob("*")) + list(
        (root / "dev/mkosi").glob("*.sh")
    )
    for phase in phases:
        if not phase.is_file():
            continue
        # Join backslash continuations FIRST. `install` and `ln` in these phases
        # wrap their destination onto the next line, and a per-line scan reads that
        # destination as a line of its own - which is how `ln -sf ... \` +
        # `"$DESTDIR/usr/bin/arlen-store"` slipped past the link rule below and put
        # `store` back among the producible ids.
        text = phase.read_text(errors="replace").replace("\\\n", " ")
        for line in text.splitlines():
            if line.lstrip().startswith("#"):
                continue
            # A SYMLINK MINTS NO ID. `path_to_app_id` resolves a peer through
            # `/proc/<pid>/exe`, which the kernel reports as the REAL file, so a
            # convenience link at `/usr/bin/arlen-<name>` pointing into an app
            # directory is never what the resolver sees - the app directory's own
            # name is (rule 3). Counting the link made `store` look producible the
            # moment `04q-store` shipped `arlen-store`, while a running store still
            # resolves to `dev.arlen.store` and an allowlist naming `store` still
            # refuses it. Only a real install under `/usr/bin` counts for rule 2.
            is_link = line.lstrip().startswith("ln ")
            for path in STAGED.findall(line):
                if path in strict:
                    out.setdefault(strict[path], path)
                elif path.startswith("/usr/bin/arlen-"):
                    if is_link:
                        continue
                    out.setdefault(path[len("/usr/bin/arlen-"):], path)
                elif path.startswith("/usr/lib/arlen/apps/"):
                    app = path[len("/usr/lib/arlen/apps/"):].split("/", 1)[0]
                    if app:
                        out.setdefault(app, path)
    return out


def main() -> int:
    names = admitted(ROOT)
    can_be = producible(ROOT)
    if not names:
        print("NOTHING WAS READ: no caller allowlists found", file=sys.stderr)
        return 2
    if not can_be:
        print("NOTHING WAS READ: no staged binaries found", file=sys.stderr)
        return 2

    deferred = deferred_daemons(ROOT)
    findings = []
    for app_id, where in sorted(names.items()):
        if app_id in can_be or app_id in NOT_PACKAGED_YET or app_id in deferred:
            continue
        findings.append(
            f"{where} admits `{app_id}`, and no path the image stages resolves to "
            f"it. Either the id is in the wrong scheme (a daemon is "
            f"`/usr/bin/arlen-<name>` and resolves to `<name>`; an app is "
            f"`/usr/lib/arlen/apps/<id>/` and resolves to `<id>`), or the caller "
            f"is not packaged yet and belongs in NOT_PACKAGED_YET with a reason."
        )

    stale = sorted(k for k in NOT_PACKAGED_YET if k in can_be)
    for app_id in stale:
        findings.append(
            f"`{app_id}` is acknowledged as not packaged and the image now stages "
            f"it ({can_be[app_id]}). Remove it from NOT_PACKAGED_YET."
        )

    if findings:
        print(
            f"{len(names)} admitted id(s) across the release allowlists, "
            f"{len(findings)} finding(s):\n",
            file=sys.stderr,
        )
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        return 1

    print(
        f"{len(names)} admitted id(s) checked against {len(can_be)} id(s) the image "
        f"can resolve: every entry is either producible or acknowledged as a caller "
        f"that is not packaged yet. An allowlist naming an id nothing can be is a "
        f"gate closed against everyone, which looks exactly like one that works."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
