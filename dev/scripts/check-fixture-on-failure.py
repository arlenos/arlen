# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that no store shows its design fixture when a real read failed.

Every app here ships fixture data so the UI can be built before its backend
exists, and every one of them loads through the same shape:

    try   { store.set(await invoke("...")) }
    catch { store.set(FIXTURE) }

The catch is written while the backend does not exist yet, when it is plainly
right. It is still there after the backend lands, and now it means something
else: a read that failed in a real session renders invented content as fact.
Four were found in one evening and each had a different local justification -
one file's own doc even said the fixture "can never" reach the live path, two
lines above the line where it did.

They were not cosmetic:

  * the screen-capture picker offered two named windows that did not exist, and
    the user picks one of them to share
  * the jobs zone invented jobs whose cancel and retry buttons act on nothing
  * the grants list said an app had no access it could not read
  * the theme grid named five themes as installed

What this looks for: inside a `catch` block, a store write (`.set` or `.update`)
in the same breath as a fixture-named constant (FIXTURE, MOCK, DEMO, SAMPLE), or
a literal carrying `mocked: true`. Both are the app stating something about the machine that it did
not learn from the machine.

What it does NOT cover, and the omission is deliberate rather than an oversight:

  * `store.set([])` in a catch. Sometimes that is the same defect wearing
    different clothes - an empty duplicate-scan reads as "no duplicates", an
    empty facet list as "this file belongs to no project" - and sometimes an
    empty list genuinely is the honest answer for that store. Telling those
    apart needs the store read, not a regex, so they are listed at the end as
    work rather than failed here. There were 9 of them when this was written.
  * A fixture reached through a helper, or assigned to a local that is set
    later.
  * Whether a store that DOES branch on `isTauri()` or `import.meta.env.DEV` put
    the fixture on the right side of it.
  * Anything outside `apps/*/src`. The `dev` fixtures and the test suites are
    supposed to be fixtures.

So a pass means no store visibly swaps a failed read for named fixture content,
not that every failure in the frontend is reported honestly.

Shown to fail before being trusted: run against `1d761f5b7~1` it names
sourcePicker, jobs and themes, which is what it was written from.

It fails today on one store, `apps/settings/src/lib/stores/models.ts`, which
belongs to somebody else's in-flight work and is left visible rather than
excused. Five entries are acknowledged with reasons.

The line the acknowledgements draw, after fixing eleven of these in one night:
a labelled sample on screen is a design choice, and someone made it deliberately;
a fixture that supplies an id, an index or a pid to a real call is a defect
whatever the label says. The system monitor was the clearest case - its rows are
labelled 'Example processes' and their ids are 1, 101, 102, 103, which is what
the Stop button passes to the backend. That is the point of committing it - the four
found by reading were a sample and not the set, and a list in the repo is worth
more than the same evening repeated four more times. It goes into CI when the
list is empty; until then it is the queue, in severity order: a printer or a
capture source the user picks and does not get, then app permissions and
capsules, then the cosmetic ones.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

CATCH = re.compile(r"\}\s*catch\b[^{]*\{", re.S)
# `.update(s => ({...s, discovered: FIXTURE.discovered}))` is the same act as
# `.set(FIXTURE)` and the first version of this check only knew the second, so
# it walked past a rescan that served invented printers on every real run.
STORE_WRITE = re.compile(r"\.(set|update)\(")
# Case matters and this got it wrong twice. The first version knew only
# `.set(FIXTURE)`; widening to `.update` found two more. This version knew only
# SHOUTING constants, and the knowledge timeline calls a lowercase `fixture()`
# helper - a week of invented activity on the app's landing view, missed because
# of capitalisation. Both times the check had inherited the spelling of the
# handful of files it was written from.
FIXTURE_NAME = re.compile(
    r"\b(FIXTURE\w*|\w*_FIXTURE|MOCK\w*|DEMO\w*|SAMPLE\w*)\b|\b(fixture|mock|demo|sample)\w*\s*\("
)
MOCKED_FLAG = re.compile(r"mocked:\s*true")
# Either guard counts as the author having separated the two sessions. Whether
# they got the branches the right way round is not something a regex settles;
# what it can see is that the question was asked at all.
DEV_GUARD = re.compile(r"import\.meta\.env\.DEV|isTauri\(")

# arlen-ui's live work. Not ours to edit, so not ours to fail on.
SKIP = ("/harness/", "/store/", "routes/ai/models")

# A store that shows fixture content on a real failure for a reason someone
# stands behind. Empty is the goal: the reason has to survive being read next to
# the sentence the user ends up seeing.
ACKNOWLEDGED: dict[str, str] = {
    'apps/text-editor/src/lib/stores/lens.ts:78': (
        "Caveat at the claim, and nothing here turns invented data into an argument. That is the line tonight's fixes drew: a labelled sample on screen is a design choice someone made, but a fixture that supplies an id, an index or a pid to a real call is a defect whatever the label says. The lens shows provenance, backlinks and project context for the open file, labelled 'Example context - not this file's real graph neighbourhood'. `openRelated` navigates rather than mutates."
    ),
    'apps/meetings/src/lib/stores/meeting.ts:119': (
        "Caveat at the claim, and nothing here turns invented data into an argument. That is the line tonight's fixes drew: a labelled sample on screen is a design choice someone made, but a fixture that supplies an id, an index or a pid to a real call is a defect whatever the label says. The meetings list is labelled above itself, and clicking a fixture meeting reaches the note path, which is labelled too."
    ),
    'apps/meetings/src/lib/stores/meeting.ts:142': (
        "Caveat at the claim, and nothing here turns invented data into an argument. That is the line tonight's fixes drew: a labelled sample on screen is a design choice someone made, but a fixture that supplies an id, an index or a pid to a real call is a defect whatever the label says. The whole note page renders from the fixture - participants, claims, transcript - under 'Example note - not a real meeting. The participants and quotes are made up.' Nothing on it writes anywhere."
    ),
    'apps/meetings/src/lib/stores/meeting.ts:290': (
        "Caveat at the claim, and nothing here turns invented data into an argument. That is the line tonight's fixes drew: a labelled sample on screen is a design choice someone made, but a fixture that supplies an id, an index or a pid to a real call is a defect whatever the label says. Same note surface, reached by summarising rather than opening."
    ),
    "apps/files/src/lib/stores/provenance.ts:99": (
        "The caveat is rendered at the claim, not on a banner elsewhere: the halo "
        "popover puts 'Sample history - not this file's real origin' directly above "
        "the chain, and the store sets `mocked` on the chain object so it travels "
        "with the data rather than with the page. Rendered and looked at, not read "
        "off the markup - and the pixels carry an argument the source did not: the "
        "chain leads with the sample's own filename in monospace, so a popover "
        "opened from `thesis-draft.md` visibly says `budget-2026.xlsx`. The reader "
        "is told it is a different file twice, once in prose and once in the data. "
        "`provenance_of` also has a real "
        "backend, so this is a genuine failure path and not the always-path. That "
        "caveat was built deliberately and verified rendering; replacing it with an "
        "empty state is a design change, not a fix, so it is not one to make at "
        "02:30 without the person who asked for it. Revisit if the halo ever gains "
        "an action, or if a sample chain about a different file stops being an "
        "acceptable answer to 'where is this from?'."
    ),
}


def catch_bodies(text: str):
    """Yield (line, body) for each catch block, matched by brace depth."""
    for m in CATCH.finditer(text):
        depth, i = 1, m.end()
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        yield text[: m.start()].count("\n") + 1, text[m.end() : i]


def main() -> int:
    findings: list[str] = []
    files = 0
    catches = 0
    for path in sorted((ROOT / "apps").rglob("*.ts")) + sorted((ROOT / "apps").rglob("*.svelte")):
        s = str(path)
        if any(k in s for k in SKIP) or "node_modules" in s or "/src/" not in s:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if "catch" not in text:
            continue
        files += 1
        for line, body in catch_bodies(text):
            catches += 1
            if DEV_GUARD.search(body):
                continue  # the dev-only fallback this is asking for
            hit = None
            if STORE_WRITE.search(body):
                hit = FIXTURE_NAME.search(body)
            hit = hit or MOCKED_FLAG.search(body)
            if not hit:
                continue
            rel = path.relative_to(ROOT)
            if f"{rel}:{line}" in ACKNOWLEDGED:
                continue
            findings.append(
                f"{rel}:{line}: a failed read falls back to `{hit.group(0).strip()}`, so "
                f"invented content renders as fact. Guard it with "
                f"`import.meta.env.DEV` and give the real session an error the "
                f"surface can show."
            )

    print(
        f"{catches} catch block(s) across {files} frontend file(s) checked for a "
        f"fixture shown after a failed read. Named fixtures only: an empty list "
        f"in a catch can be the honest answer for some stores and a false claim "
        f"for others, which needs the store read rather than this."
    )
    if findings:
        print("\nstores that answer a failed read with invented content:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
