#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Every component is reachable from a route.

The frontend's version of `no_new_daemon_module_is_unreachable`: that check asks
what IMPORTS a module, this one asks what RENDERS a component. Same failure, and
it is a quiet one - the file compiles, its types check, a harness can even mount
it and photograph it, and nothing in the product shows it to anybody.

Written because it happened. `Composer.svelte` stopped being rendered when xterm.js
took the whole terminal, and two fixes were committed to it - including one
"verified" with a screenshot of the component mounted directly, which proves a
component works and says nothing about whether a surface renders it.

REACHABILITY, not "is it imported anywhere". Two orphans importing each other are
still orphans, so the graph is walked from the routes: `+page.svelte`,
`+layout.svelte` and `+error.svelte` are the only roots a person can arrive at.
`sdk/ui-kit` has no routes of its own and is reached through the apps that use it,
which is the honest picture of a library.

WHAT IT CANNOT SEE, and therefore what the exception list is for: a component
chosen at runtime from a registry (`<svelte:component this={map[kind]} />`), and a
path built from a variable. Both resolve to a node this walk never visits, so they
are listed with a reason rather than silently trusted.
"""

import re
import sys
from pathlib import Path

REPO = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
OWN_TREE = len(sys.argv) <= 1

SKIP = ("node_modules", "/.svelte-kit/", "/build/", "/dist/", "/target/")

# A route file is a root: it is what a person arrives at.
ROOT_NAMES = ("+page.svelte", "+layout.svelte", "+error.svelte")

# Components reached in a way this walk cannot follow, or deliberately kept while
# unrendered. An entry is a claim someone checked, not a place to park a file.
KNOWN: dict[str, str] = {
    # Each entry names the surface it waits for, which is also the condition that
    # retires it: when that surface exists and renders the component, the walk
    # reaches it and the entry has to go. An exception with no condition is just a
    # place to park a file.
    "apps/desktop-shell/src/lib/components/settings/PermissionScope.svelte": (
        "a part of the capability browser (LCG-R7), planned and unbuilt. Retires "
        "when that surface renders it"
    ),
    "apps/desktop-shell/src/lib/components/settings/PermissionsPanel.svelte": (
        "a part of the capability browser (LCG-R7), planned and unbuilt. Retires "
        "when that surface renders it"
    ),
    "apps/settings/src/lib/components/appearance/AccentPicker.svelte": (
        "ahead of the appearance surface (`appearance-surface.md`, deferred but "
        "decided). Retires when that page renders it"
    ),
    "apps/settings/src/lib/components/appearance/BorderColorPicker.svelte": (
        "ahead of the appearance surface (`appearance-surface.md`, deferred but "
        "decided). Retires when that page renders it"
    ),
    "sdk/ui-kit/src/lib/components/a11y-kitchen.svelte": (
        "the kit's accessibility demo: rendered by ui-kit's own `_a11y` harness "
        "route and asserted by `a11y.test.ts`. Deliberately not exported, because "
        "it is a test surface rather than a part apps compose with"
    ),
}


# Modules nothing reaches, carried with a reason. Same rule as `KNOWN`: the entry
# names the condition that retires it.
KNOWN_MODULES: dict[str, str] = {
    "apps/desktop-shell/src/lib/invokeWithTimeout.ts": (
        "a hard timeout around `invoke`, written for the startup path after a hung "
        "backend froze the shell. Nothing calls it, which means nothing is "
        "protected - but it is infrastructure someone reached for once and will "
        "again. Retires when a startup call uses it, or when the decision is taken "
        "that a hung backend is handled another way"
    ),
    "apps/settings/src/lib/stores/compositor.ts": (
        "the compositor `[layout]` config store, left behind when its consumer "
        "moved. Retires when the workspaces page reads through it again, or when "
        "someone confirms the page's own path is the only one wanted"
    ),
    "sdk/ui-kit/src/lib/transitions.ts": (
        "kit transitions, not exported from the barrel and used by nothing. "
        "Retires when a kit component animates through them or the barrel exports "
        "them"
    ),
    "apps/desktop-shell/src/lib/components/ui/index.ts": (
        "a shadcn barrel the shell does not import from - components take the deep "
        "path. Retires when the barrel is the import route or is removed"
    ),
    "apps/settings/src/lib/components/ui/index.ts": (
        "the same shadcn barrel, same story, in Settings"
    ),
    "apps/desktop-shell/src/lib/modules/index.ts": (
        "the module-host barrel, ahead of the Tier-2 module surface. Retires when "
        "that surface imports it"
    ),
    "apps/terminal/src/lib/keymap.ts": (
        "the terminal's key table, orphaned by the xterm.js cutover, which owns key "
        "handling now. Retires by deletion once the cutover's leftovers are swept "
        "as a batch"
    ),
    "apps/terminal/src/lib/live-region.ts": (
        "the old renderer's rule for which grid rows are the LIVE region (the "
        "running command or the idle prompt, as against finished blocks). Nothing "
        "to do with ARIA despite the name - I read it as an announcer once and "
        "wrote that here, which is why this entry now says what the file is. "
        "Orphaned by the xterm cutover, which owns the grid. Retires by deletion "
        "with the rest of that cutover's leftovers"
    ),
}

# `utils.ts` per app: eleven near-identical shadcn scaffold copies of `cn()`, all
# unimported because components take the kit's. They are scaffold rather than code
# someone wrote, and `npx shadcn-svelte` regenerates them, so deleting all eleven
# is a decision about the scaffold and not a cleanup. Excluded by name and
# recorded here so the report is not eleven lines of the same sentence.
SCAFFOLD = ("utils.ts",)


def sources(repo: Path):
    """Every frontend source file that can import or be imported."""
    out = []
    for root in [repo / "apps", repo / "sdk" / "ui-kit"]:
        if not root.is_dir():
            continue
        for ext in ("*.svelte", "*.ts", "*.js"):
            for f in root.rglob(ext):
                if not any(s in str(f) for s in SKIP):
                    out.append(f)
    return out


IMPORT = re.compile(r"""(?:from|import)\s*\(?\s*["']([^"']+)["']""")


def app_of(path: Path, repo: Path):
    """The package root a file belongs to, for resolving `$lib`."""
    rel = path.relative_to(repo).parts
    if rel[0] == "apps" and len(rel) > 1:
        return repo / "apps" / rel[1]
    if rel[:2] == ("sdk", "ui-kit"):
        return repo / "sdk" / "ui-kit"
    return None


def resolve(spec: str, importer: Path, repo: Path):
    """A specifier to a file on disk, or None when it leaves the tree."""
    base = None
    if spec.startswith("."):
        base = (importer.parent / spec).resolve()
    elif spec.startswith("$lib/"):
        app = app_of(importer, repo)
        if app:
            base = app / "src" / "lib" / spec[len("$lib/"):]
    elif spec.startswith("@arlen/ui-kit"):
        tail = spec[len("@arlen/ui-kit"):].lstrip("/")
        base = repo / "sdk" / "ui-kit" / "src" / "lib" / tail if tail else None
    if base is None:
        return None
    # A bare specifier may name the file, the file without its extension, or a
    # directory with an index. Missing this is how a whole library reads as
    # unreached.
    #
    # `.js` -> `.ts` is not a nicety: SvelteKit's convention is to import the
    # EMITTED name, so `$lib/quicksettings/grid.js` is `grid.ts` on disk. Without
    # this the walk stopped at the panel and reported nine quick-settings tiles as
    # rendered by nobody, when a registry two hops further along renders every one
    # of them. A check that cries wolf about a live surface gets switched off.
    swapped = Path(str(base)[:-3] + ".ts") if str(base).endswith(".js") else None
    for cand in (
        base,
        swapped,
        Path(str(base) + ".ts"),
        Path(str(base) + ".js"),
        Path(str(base) + ".svelte"),
        base / "index.ts",
        base / "index.js",
        base / "index.svelte",
    ):
        if cand and cand.is_file():
            return cand.resolve()
    return None


def repo_of(_f: Path) -> Path:
    """The repository root, so route parts can be read without absolute noise."""
    return REPO


def main() -> int:
    files = sources(REPO)
    if not files:
        print(f"NOTHING WAS READ: no frontend sources under {REPO}", file=sys.stderr)
        return 2

    text = {f: f.read_text(encoding="utf-8", errors="replace") for f in files}
    edges = {
        f: {r for r in (resolve(s, f, REPO) for s in IMPORT.findall(t)) if r}
        for f, t in text.items()
    }

    # A HARNESS route is not a surface. Every screenshot route in this tree is
    # `_`-prefixed (`_rendertest`, `_qstest`, `_nettest`), and counting them as
    # roots defeats the check exactly where it is needed: `Composer.svelte` is
    # reached from the harness I wrote for it and from nowhere a person can go,
    # which is the case this exists to catch. Reachable-only-from-a-harness IS
    # unreached.
    roots = [
        f
        for f in files
        if f.name in ROOT_NAMES
        and "/src/routes/" in str(f)
        and not any(part.startswith("_") for part in f.relative_to(repo_of(f)).parts)
    ]
    # A LIBRARY's entry points are its exports, the way an app's are its routes.
    # `sdk/ui-kit` has no product routes of its own, and a component it exports is
    # doing its job whether or not an app happens to use it today - reporting
    # those would bury the real finding under a shelf of available parts, and a
    # check nobody can read gets switched off. Unexported AND unreached is still
    # an orphan, which is what stays covered.
    roots += [
        f
        for f in files
        if f.name in ("index.ts", "index.js")
        and str(f).startswith(str(REPO / "sdk" / "ui-kit" / "src" / "lib"))
    ]
    if not roots:
        print(f"NOTHING WAS READ: no route files under {REPO}", file=sys.stderr)
        return 2

    seen, stack = set(roots), list(roots)
    while stack:
        cur = stack.pop()
        for nxt in edges.get(cur, ()):
            if nxt not in seen:
                seen.add(nxt)
                stack.append(nxt)

    components = [f for f in files if f.suffix == ".svelte" and f.name not in ROOT_NAMES]
    unreached = sorted(
        str(f.relative_to(REPO)) for f in components if f not in seen
    )

    # A store is the same question with a different extension, and the walk was
    # already visiting these files - it just never reported them. Found by
    # following a store holding `{ charging: false, level: 72 }`: nothing had
    # imported `applets.ts` since Phase 4, so a battery level nobody measured sat
    # in the tree looking like state. Its neighbour's doc said it was "read
    # directly by WorkspaceIndicator.svelte", which had stopped being true.
    #
    # Reported separately from components, because the fix differs: an unrendered
    # component usually wants wiring, an unimported module usually wants deleting.
    modules = [
        f
        for f in files
        if f.suffix in (".ts", ".js")
        and "/src/lib/" in str(f)
        and not f.name.endswith((".test.ts", ".test.js", ".d.ts"))
        and f.name not in SCAFFOLD
    ]
    unreached_modules = sorted(
        str(f.relative_to(REPO)) for f in modules if f not in seen
    )

    problems, carried = [], []
    for rel in unreached:
        if rel in KNOWN:
            carried.append(f"{rel}: {KNOWN[rel]}")
            continue
        problems.append(
            f"{rel}: no route reaches it.\n"
            f"    It compiles and it renders for nobody. Wire it up, or add it to "
            f"KNOWN with the reason it waits - and if a registry renders it by "
            f"name, say which one."
        )

    for rel in unreached_modules:
        if rel in KNOWN_MODULES:
            carried.append(f"{rel}: {KNOWN_MODULES[rel]}")
            continue
        problems.append(
            f"{rel}: nothing a route reaches imports it.\n"
            f"    Whatever it holds is not state, it is a file. Delete it, or add "
            f"it to KNOWN_MODULES with the reason it waits."
        )

    if OWN_TREE:
        for rel in sorted(KNOWN_MODULES):
            if rel not in unreached_modules:
                problems.append(
                    f"{rel}: listed as unimported, and something reaches it now. "
                    f"Drop the entry."
                )
        for rel in sorted(KNOWN):
            if rel not in unreached:
                problems.append(
                    f"{rel}: listed as unreached, and a route reaches it now. Drop "
                    f"the entry; an exception that outlived its reason reads as "
                    f"coverage."
                )

    if carried:
        print("carried, with a reason (see KNOWN):")
        for line in carried:
            print(f"  {line}")
        print()

    if problems:
        print("components no route reaches:")
        for p in problems:
            print(f"  {p}")
        return 1

    print(
        f"OK: {len(components)} component(s) and {len(modules)} lib module(s), "
        f"every one reachable from a route"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
