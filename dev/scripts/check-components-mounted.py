#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A component nothing mounts is named here, or it is a finding.

An unmounted component is not a broken one. It compiles, `svelte-check` is happy,
its strings sit in the catalogue and count toward the i18n baselines, and every
sweep in this tree renders zero pixels of it - so nothing measures it and nothing
notices when the thing it was written for changes underneath it. It reads, from
inside the repository, exactly like a feature.

Three of them stand today and all three are real work: the shell's read-only
permissions panel (Settings grew its own privacy page), and the Settings
appearance pickers for the accent and the window border colour. `ACCENT_PRESETS`
is exported for one of them and consumed by nothing else, so the dead chain is two
deep.

Whether they come back or come out is Tim's call and it has been open since the
count was seven. What this stops is the count going up again without anybody
seeing it: a new orphan fails, and adding it to the list below means writing down
why it is there.

    unmounted   a `.svelte` file no other source imports or tags

Route files are skipped: `+page.svelte` and friends ARE the entry points, mounted
by the router rather than by an import.
"""

import os
import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

SKIP_DIRS = {"node_modules", ".svelte-kit", "build", "target", ".git", "dist"}

# Components that are deliberately unmounted, with why. MAY SHRINK, MAY NOT GROW.
CARRIED = {
    "apps/desktop-shell/src/lib/components/settings/PermissionsPanel.svelte": (
        "a read-only permission list from before Settings had its own privacy and "
        "apps pages; superseded rather than broken, and whether it comes out is a "
        "product call recorded in coder-reports"
    ),
    "apps/settings/src/lib/components/appearance/AccentPicker.svelte": (
        "the compact accent swatch row; the colours page draws accents another way "
        "now. `ACCENT_PRESETS` exists for this component and nothing else"
    ),
    "apps/settings/src/lib/components/appearance/BorderColorPicker.svelte": (
        "the window-border colour picker, same story as the accent one beside it"
    ),
}


def sources():
    """Every Svelte and TypeScript source under the app and kit trees."""
    roots = [p for p in sorted((ROOT / "apps").glob("*/src")) if p.is_dir()]
    kit = ROOT / "sdk/ui-kit/src"
    if kit.is_dir():
        roots.append(kit)
    for root in roots:
        for base, dirs, files in os.walk(root):
            dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            for name in files:
                if name.endswith((".svelte", ".ts")):
                    yield Path(base) / name


def main() -> int:
    files = list(sources())
    if not files:
        print("!! NOTHING WAS READ: no sources under apps/*/src or sdk/ui-kit/src",
              file=sys.stderr)
        return 2

    blob = "\n".join(
        f.read_text(encoding="utf-8", errors="replace") for f in files
    )

    components = [f for f in files if f.suffix == ".svelte" and not f.name.startswith("+")]
    if not components:
        print("!! NOTHING WAS READ: no components found", file=sys.stderr)
        return 2

    problems = []
    carried = 0
    for comp in sorted(components):
        rel = str(comp.relative_to(ROOT))
        name = comp.stem
        # An import by path (quoted, with or without the extension) or a tag.
        imported = re.search(r'["\'/]%s(\.svelte)?["\']' % re.escape(name), blob)
        tagged = re.search(r"<%s[\s/>]" % re.escape(name), blob)
        if imported or tagged:
            if rel in CARRIED:
                problems.append(
                    f"{rel} is carried as unmounted and something mounts it now; "
                    f"delete the entry"
                )
            continue
        if rel in CARRIED:
            carried += 1
            continue
        problems.append(
            f"{rel} is imported by nothing and tagged nowhere, so no route renders it "
            f"and no sweep measures it. Mount it, delete it, or write down why it is "
            f"here."
        )

    # Only in the real tree: a fixture tree legitimately has none of these, and
    # reporting all three there would make every control fail for the wrong
    # reason. The marker is this file, which only the repository has.
    if (ROOT / "dev/scripts/check-components-mounted.py").is_file():
        for stale in sorted(CARRIED):
            if not (ROOT / stale).is_file():
                problems.append(f"{stale} is carried here and does not exist; delete the entry")

    if problems:
        print("Components nothing mounts:", file=sys.stderr)
        for p in problems:
            print(f"  - {p}", file=sys.stderr)
        return 1

    print(
        f"{len(components)} component(s); every one of them is imported or tagged "
        f"somewhere ({carried} carried with a reason)."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
