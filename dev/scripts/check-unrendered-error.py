# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a store which records a failure is rendered by something.

An app that catches a failed read and records it - `unavailable.set(true)`,
`error.set(String(e))` - has done the hard half: it knows the difference between
"you have none" and "we could not ask". The flag is only worth anything if a
component reads it. When nothing does, the app holds the honest answer and shows
the dishonest one, and it does so silently, forever, because there is no symptom
to notice.

That is not hypothetical. `knowledge`'s `savedUnavailable` was set by
`loadSavedSearches`'s catch and read by nobody, so the Searches place answered a
failed read with "No saved searches yet." - a statement about the person's own
data, made after failing to look at it. The flag saying otherwise was two lines
from the code that set it. Found on 16 August by scanning for exactly this, after
finding the same shape by hand in four other places.

What this looks for, in two shapes:

  1. An exported `writable` in `apps/*/src` whose name reads like a failure
     (`error`, `unavailable`, `failed`), which no `.svelte` file in that app
     mentions.

  2. A store typed by a state object that CARRIES an `error` field - the
     `{data, loading, error}` shape most of settings uses, and the one
     `ConfigUnavailable` exists to render - where no component reads
     `$store.error`.

Shape 2 was added after shape 1 missed a live one. `keyboard/shortcuts` answered
a failed read with `No bindings match ""`, pointing at an empty search box, while
`keybindings.ts` held the failure in its state the whole time. Nothing about that
store is unusual; the flag simply was not a top-level writable, so the first rule
could not see it. Found by rendering the page, which is the expensive way.

What it does NOT cover, deliberately:

  * Whether the component that mentions it renders it USEFULLY. A name appearing
    inside a component is the floor, not the proof; `check-fixture-on-failure`
    has the same limit for the same reason, and reading a render is a judgement.
  * A failure recorded in a plain `$state` inside one component rather than an
    exported store. That is the shape of the night-light bug, and it is
    `check-optimistic-write`'s subject.
  * A store consumed only through a derived store. If the derived one reaches a
    component the flag IS surfaced, and this would call that a finding. None
    exist today; the acknowledgement below is where one would go.

So a pass means every recorded failure has a reader, not that every failure is
reported well.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

#: An exported writable whose name says it holds a failure.
FAILURE_STORE = re.compile(
    r"export const (\w*(?:[eE]rror|[uU]navailable|[fF]ailed)\w*)\s*=\s*writable"
)

#: A state object declaration, so its body can be searched for an `error` field.
STATE_TYPE = re.compile(r"(?:interface|type)\s+(\w+)\s*(?:=\s*)?\{([^}]*)\}", re.S)

#: `export const thing: Readable<ThingState>` - the store, named by its state.
TYPED_STORE = re.compile(r"export const (\w+)\s*:\s*\w+<\s*(\w+)")

#: An `error` field in a state body. Not `errorCount`, not a nested `onError`.
ERROR_FIELD = re.compile(r"^\s*error\s*\??\s*:", re.M)

#: Keys are `app:store`. A deliberate silence belongs here WITH its reason, so
#: the next reader can disagree with the decision rather than rediscover the
#: finding.
ACKNOWLEDGED: dict[str, str] = {
    "desktop-shell:themeError": (
        "A theme that fails to load leaves the shell drawing with its built-in "
        "tokens, which is visible without being told. The only place to surface "
        "it is the top bar, and a banner across every screen about a cosmetic "
        "fallback is a decision about how loud the shell should be - not one to "
        "make by wiring a flag because a check asked for it."
    ),
}


def main() -> int:
    # `iterdir()` on a missing directory raises, and a traceback is not a
    # refusal: the caller sees a non-zero exit either way and cannot tell "this
    # tree has no apps" from "the checker broke". Its own control caught this.
    root_apps = ROOT / "apps"
    apps = (
        sorted(p for p in root_apps.iterdir() if (p / "src").is_dir())
        if root_apps.is_dir()
        else []
    )
    if not apps:
        print(f"NOTHING WAS READ: no apps under {ROOT / 'apps'}", file=sys.stderr)
        return 2

    findings: list[str] = []
    checked = 0
    for app in apps:
        src = app / "src"
        stores: dict[str, Path] = {}
        for p in src.rglob("*.ts"):
            for m in FAILURE_STORE.finditer(p.read_text(encoding="utf-8", errors="replace")):
                stores[m.group(1)] = p
        # No early exit on an empty `stores`: the second shape below is
        # independent of the first, and an app can have only the second. Its
        # control caught exactly that, on the first run.
        markup = "".join(
            p.read_text(encoding="utf-8", errors="replace") for p in src.rglob("*.svelte")
        )
        for name, where in sorted(stores.items()):
            checked += 1
            key = f"{app.name}:{name}"
            if name in markup or key in ACKNOWLEDGED:
                continue
            rel = where.relative_to(ROOT)
            findings.append(
                f"{rel}: `{name}` records a failure that no component reads, so the "
                f"app knows the read failed and shows the empty-looking answer instead"
            )

        # Shape 2: the failure is a field on the store's state.
        for p in src.rglob("*.ts"):
            text = p.read_text(encoding="utf-8", errors="replace")
            carriers = {
                m.group(1)
                for m in STATE_TYPE.finditer(text)
                if ERROR_FIELD.search(m.group(2))
            }
            if not carriers:
                continue
            for m in TYPED_STORE.finditer(text):
                name, state = m.group(1), m.group(2)
                if state not in carriers:
                    continue
                checked += 1
                key = f"{app.name}:{name}"
                if f"{name}.error" in markup or key in ACKNOWLEDGED:
                    continue
                rel = p.relative_to(ROOT)
                findings.append(
                    f"{rel}: `{name}` carries an error in `{state}` that no component "
                    f"reads, so a failed read renders as an ordinary empty result"
                )

    print(
        f"{checked} failure-recording store(s) checked for a reader in {len(apps)} app(s), "
        f"{len(ACKNOWLEDGED)} acknowledged with a reason. A name reaching markup is the "
        f"floor: whether it is rendered where the claim is made needs a person."
    )
    if findings:
        print("\nfailures the app records and never says:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
