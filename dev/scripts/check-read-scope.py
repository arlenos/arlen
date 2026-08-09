# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app's permission profile grants the labels its queries read.

The knowledge daemon resolves a caller's read scope from its profile and refuses
any query naming a label outside it. A profile that is merely INCOMPLETE
therefore does not fail loudly - the query is denied, the app's error path runs,
and what the user sees is a feature that found nothing. On 10 August that was
true of two shipped apps at once: the file manager's Ask facets read
`MATCH (a:App)` with no `system.App` grant, and the shell's activity strip read
`MATCH (e:Event)` with no `system.Event` grant, falling back to `empty_response()`
and drawing a flat week. Neither says "denied" anywhere a person would look.

This is the read-side twin of the tier check that found three graph writers sitting
below the write socket's doorway the same night. Both are the same question: does
the assembly agree with the code, or only the code with itself?

What this does NOT cover:

  * Fields. A profile granting `system.File.id` while the query returns
    `f.path` is a narrower version of the same bug, and matching field names
    across a format string is guesswork this cannot do honestly. Labels are
    exact and are where the denial actually bites.
  * Labels named outside a `MATCH (x:Label` - a traversal written as
    `-[:REL]->(m:Label)`, or a label built at run time. Under-reads rather than
    invents: it can miss a missing grant, never fabricate one.
  * Apps with no profile at all. `check-app-profiles.py` owns that; here an app
    with no profile is skipped rather than reported twice.
  * Whether the granted label EXISTS in the graph schema. A grant for a label
    nothing writes is inert, not a lie.

Shown to fail before being trusted: drop `system.App.id` from
`dev.arlen.files.toml` and it names the app, the label and the query.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[2]
PROFILES = ROOT / "dev/mkosi/mkosi.extra/var/lib/arlen/permissions/0"

# `MATCH (n:Label`, the shape every live read in the tree uses.
MATCH = re.compile(r"MATCH\s*\(\s*\w*\s*:\s*(\w+)")
GRANT = re.compile(r'"system\.(\w+)')

# Labels an app queries deliberately without a grant, and why. An entry here is
# a claim that the denial is intended; it is not a place to park work.
ACCEPTED: dict[str, dict[str, str]] = {}


def app_sources(app: str) -> list[pathlib.Path]:
    """The Rust an app's own process runs. Its frontend cannot query the graph."""
    out = []
    for sub in ("src-tauri/src", "core/src"):
        d = ROOT / "apps" / app / sub
        if d.is_dir():
            out.extend(p for p in d.rglob("*.rs"))
    return out


def main() -> int:
    findings: list[str] = []
    checked = 0

    for profile in sorted(PROFILES.glob("dev.arlen.*.toml")):
        app = profile.stem.removeprefix("dev.arlen.")
        sources = app_sources(app)
        if not sources:
            continue
        granted = set(GRANT.findall(profile.read_text()))
        accepted = ACCEPTED.get(app, {})
        checked += 1

        for src in sources:
            text = src.read_text(errors="replace")
            for label in sorted(set(MATCH.findall(text))):
                if label in granted or label in accepted:
                    continue
                rel = src.relative_to(ROOT)
                findings.append(
                    f"apps/{app}: reads `MATCH (:{label})` in {rel} but "
                    f"{profile.name} grants no `system.{label}`. The daemon denies "
                    f"the query and the app renders whatever its error path says - "
                    f"usually nothing, which reads as an empty graph."
                )

    print(
        f"{checked} app(s) checked that every queried label is granted. "
        f"Labels only: a missing FIELD is the same bug one size smaller and is "
        f"not covered here."
    )
    if findings:
        print("\nqueried without a grant:\n")
        for f in sorted(set(findings)):
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
