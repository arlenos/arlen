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
#:
#: The first three words were the ones the tree happened to use in August. Seven
#: more stores have since been written as `openFailure`, `searchRefusals`,
#: `launchFailure`, `meetingsFailure`, `historyOnlyFailures` - all of them
#: recording a failure, none of them matching, and all of them read today by luck
#: rather than by this. A gate that only knows the words its author last wrote is
#: the recurring fault in this directory, so the vocabulary is widened before the
#: next one is silent instead of after.
FAILURE_STORE = re.compile(
    r"export const (\w*(?:[eE]rror|[uU]navailable|[fF]ail|[rR]efus|[dD]enied|[bB]locked"
    r"|[oO]ffline|[bB]roken|[nN]otice)\w*)\s*=\s*writable"
)

#: A state object declaration, so its body can be searched for an `error` field.
STATE_TYPE = re.compile(r"(?:interface|type)\s+(\w+)\s*(?:=\s*)?\{([^}]*)\}", re.S)

#: `export const thing: Readable<ThingState>` - the store, named by its state.
TYPED_STORE = re.compile(r"export const (\w+)\s*:\s*\w+<\s*(\w+)")

#: An `error` field in a state body. Not `errorCount`, not a nested `onError`.
ERROR_FIELD = re.compile(r"^\s*error\s*\??\s*:", re.M)


def is_read_in(name: str, markup: str) -> bool:
    """Does `name` appear in the markup AS A NAME, rather than inside a longer one?

    A plain `name in markup` passes on any word that happens to contain it, and
    one store here is short enough for that to matter: `mouse`. Settings' markup
    is full of `onmousedown`, `onmouseenter`, `mouseZoom` and
    `enable_mouse_zoom_shortcuts`, so the substring test answers yes whether or
    not anything reads the store. It answers yes today for the right reason - the
    page does draw `$mouse` - which is exactly why it was worth checking: a test
    that passes for the wrong reason passes just as loudly once the right reason
    goes away.

    A store reaches markup as `$name`, or bare inside a script block, so the `$`
    is optional and only an identifier character on either side disqualifies a
    hit.
    """
    return re.search(rf"(?<![A-Za-z0-9_])\$?{re.escape(name)}(?![A-Za-z0-9_])", markup) is not None

#: Keys are `app:store`. A deliberate silence belongs here WITH its reason, so
#: the next reader can disagree with the decision rather than rediscover the
#: finding.
ACKNOWLEDGED: dict[str, str] = {
    # NOT argued for - recorded, and its owner should read it. `apps/settings` is
    # arlen-ui's app and this lane does not edit it. `capsuleNotice` is written by
    # `revokeCapsule`'s catch and read by nothing, under a comment that says the
    # row "goes back and says why". The row does go back. The why goes into this
    # store. So a refused revoke shows a share the person just tried to stop,
    # still listed, with nothing said - on the surface that answers who can read a
    # slice of their graph.
    "settings:capsuleNotice": (
        "arlen-ui's app; reported to its owner rather than edited from this lane. "
        "A refused capsule revoke restores the row and says nothing, which on this "
        "surface reads as a share that is still live for no stated reason"
    ),
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
    # Frontends under `daemons/` are frontends: the file picker is the dialog
    # every app borrows, and a failure it records and never draws is as silent
    # there as anywhere.
    apps += sorted(
        p
        for p in (ROOT / "daemons").glob("*/*")
        if (p / "src").is_dir() and (p / "package.json").is_file()
    )
    if not apps:
        print(f"NOTHING WAS READ: no frontend under {ROOT / 'apps'} or {ROOT / 'daemons'}", file=sys.stderr)
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
            if is_read_in(name, markup) or key in ACKNOWLEDGED:
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
                if is_read_in(f"{name}.error", markup) or key in ACKNOWLEDGED:
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
