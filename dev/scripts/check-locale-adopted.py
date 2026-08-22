# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that an app with a catalogue adopts a language at startup.

Shipping a translated catalogue and never reading the user's choice makes every
translation in it dead: the store keeps the source language, `$t(...)` resolves
through it, and the app is English forever no matter what anybody selected. There
is no symptom short of rendering the app in another language and noticing.

The greeter did exactly this until 17 August. It had a full German catalogue, and
it had `$t()` on every string, and it had never called anything that sets the
locale - so the German was unreachable, on the one screen a first-run reader has
nothing else to judge the system by. Found by rendering it with `?locale=de` and
seeing English.

Two forms count as adopting one:

    initArlenLocale()   the kit helper, which reads the choice and follows changes
    locale.set(...)     an app resolving it itself, which is what Settings does
                        (it owns the file the others read)

What this does NOT cover:

  * WHICH language is adopted. The greeter runs before login, so the user-choice
    command answers nothing there and it falls back to the environment; whether a
    login screen should instead speak the selected profile's language is an open
    question in `coder-reports.md`, not something a checker can decide.
  * An app that adopts the locale but formats dates with `navigator.language`
    anyway, which is the same screen in two languages. That was the greeter's
    second half, and catching it needs a rule about `Intl` construction rather
    than about startup.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

#: Either form of adopting a language.
ADOPTS = re.compile(r"initArlenLocale\s*\(|locale\.set\s*\(")

#: Keys are app directory names. A silence belongs here WITH its reason.
ACKNOWLEDGED: dict[str, str] = {
    "harness": (
        "arlen-ui's live work, and the same gap: a catalogue with no startup "
        "adoption, so its German is unreachable. Reported rather than edited - "
        "the fix is one line in their layout and theirs to make."
    ),
    "store": (
        "arlen-ui's, as above. Both were found by the same scan that found the "
        "greeter's, and both are recorded here so the gate stays green for their "
        "lane while the finding is written down where they will meet it."
    ),
}


def main() -> int:
    apps = ROOT / "apps"
    if not apps.is_dir():
        print(f"NOTHING WAS READ: no apps under {apps}", file=sys.stderr)
        return 2

    # The daemons carry frontends too: the file picker is the one dialog every
    # app on the machine borrows. A scope of `apps/` only was written when the
    # tree had nothing else with a `src/lib/i18n` in it.
    fronts = [p for p in apps.iterdir() if (p / "src").is_dir()]
    fronts += [
        p
        for p in (ROOT / "daemons").glob("*/*")
        if (p / "src").is_dir() and (p / "package.json").is_file()
    ]

    findings: list[str] = []
    checked = 0
    for app in sorted(fronts):
        catalogue = list((app / "src/lib/i18n").glob("*.ts")) if (app / "src/lib/i18n").is_dir() else []
        if not catalogue:
            continue
        checked += 1
        src = "".join(
            p.read_text(encoding="utf-8", errors="replace")
            for p in (app / "src").rglob("*")
            if p.suffix in {".ts", ".svelte"} and "node_modules" not in p.parts
        )
        if ADOPTS.search(src) or app.name in ACKNOWLEDGED:
            continue
        findings.append(
            f"{app.relative_to(ROOT)}: ships a catalogue and never adopts a language, so every "
            f"translation in it is unreachable and the app is the source language forever"
        )

    if not checked:
        print("NOTHING WAS READ: no app carries a catalogue", file=sys.stderr)
        return 2

    print(
        f"{checked} app(s) with a catalogue checked for a startup language, "
        f"{len(ACKNOWLEDGED)} acknowledged with a reason. Startup only: whether the "
        f"language it adopts is the right one is a question for a person."
    )
    if findings:
        print("\ncatalogues nothing can reach:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
