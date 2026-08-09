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
OR a `return`, in the same breath as a fixture-named constant (FIXTURE, MOCK,
DEMO, SAMPLE), or a literal carrying `mocked: true`. All of them are the app
stating something about the machine that it did not learn from the machine.

The `return` half was added on 9 August, after the knowledge app's provenance
read turned out to be exactly this shape and invisible: a plain function whose
catch returned a five-hop lineage under the user's real filename, with no store
write anywhere for the check to find.

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

The shape to copy, if you are writing one of these: `apps/terminal/src/routes/+page.svelte`.
It branches on `$sessionsLoaded && $sessionsError` BEFORE the empty case, gives
the failure its own title and hint out of the catalogue, and puts a Try again
button next to them. Three states, kept apart: not loaded yet, loaded and failed,
loaded and empty. Rendered with no backend it says "Can't reach the shell
backend / The terminal engine did not answer" and nothing else - no fixture, no
raw exception, no claim about the machine. Every defect found on the night of
8 August was some collapse of those three states into two.

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

# An argument overrides the tree to scan, so this check can be pointed at a
# throwaway one and shown to fail. It could not be, before 9 August: its own
# docstring proved it by naming an old commit to run it against, which is a
# demonstration nobody repeats. `test-check-fixtures.mjs` now runs it over both
# the shape that must be reported and the neighbour that must not.
ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

CATCH = re.compile(r"\}\s*catch\b[^{]*\{", re.S)
# `.update(s => ({...s, discovered: FIXTURE.discovered}))` is the same act as
# `.set(FIXTURE)` and the first version of this check only knew the second, so
# it walked past a rescan that served invented printers on every real run.
STORE_WRITE = re.compile(r"\.(set|update)\(")
# A catch that RETURNS the fixture instead of writing a store is the same act by
# a different route, and the check walked past every one of them until the
# knowledge app's provenance read turned out to be one: `provenanceFor` answered
# a failed read with a five-hop lineage it had made up, under the user's real
# filename. Nothing was `.set` anywhere, so nothing was reported.
RETURNS = re.compile(r"\breturn\b")
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
# A fixture that reaches the screen without passing through a catch at all.
#
# Added 9 August, from the one the catch pass could never have seen: the viewers
# app answered `initial_file` returning null with a bare `return`, which left the
# component on its default branch - and its default branch was the mock. A
# shipped window opened on nothing showed a track called "Nightswim", a waveform
# and a playhead at 1:13 of 3:40, with no caveat anywhere, and no `catch` and no
# `.set` for the pass above to find. It was found by reading the branch behind a
# screenshot, which does not scale.
#
# The signal that does scale is the name: data called `audioMock` or `FIXTURE`
# rendered straight into markup is a component whose resting state is invented.
# Deliberately not matching a bare lowercase `demo`, which is a mode string in
# several files and would drown the real ones.
#
# `...Mocked` is excluded, and getting that wrong first was instructive: the
# first version flagged `meetingsMocked`, `grantsMocked`, `sentinelMocked` and
# four more, which are the flags that put "showing example data" ON the screen.
# It was reporting the honesty as the dishonesty. The codebase has a convention
# worth stating: `xMock` / `XFIXTURE` is the invented data, `xMocked` is the
# boolean admitting it is showing. Only the first belongs in markup.
MARKUP_FIXTURE = re.compile(
    r"\b\w*(?:Mock|Fixture|Sample)(?!ed\b)\w*\b|\b(?:FIXTURE|MOCK|SAMPLE|DEMO)_?\w*\b"
)
SVELTE_COMMENT = re.compile(r"<!--.*?-->", re.S)
SVELTE_STYLE = re.compile(r"<style[^>]*>.*?</style>", re.S)
# Either guard counts as the author having separated the two sessions. Whether
# they got the branches the right way round is not something a regex settles;
# what it can see is that the question was asked at all.
DEV_GUARD = re.compile(r"import\.meta\.env\.DEV|isTauri\(")

# In CI since 9 August. It was the one check in `dev/scripts/` that no runner
# called - found by asking the mechanical question rather than remembering: 26
# check scripts, 23 in the workflow. The other two absentees are honest, and are
# named here so nobody has to re-derive it: `check-image-contents.sh` needs a
# built image, and `check-sensing-vectors.sh` skips when the compositor is not
# checked out, so on a runner it would pass without looking at anything.
#
# The `file:line` keys below drift, which is the cost of admitting it here: the
# first run after this note found its own lens.ts entry twenty lines stale,
# because tonight's edits to that file moved it. That is the check working.

# arlen-ui's live work. Not ours to edit, so not ours to fail on.
SKIP = (
    "/harness/",
    "/store/",
    "routes/ai/models",
    # The store BEHIND that route, and reached from nowhere else - both its
    # importers are `routes/ai/models/**`. Skipping the route while checking its
    # only store left the same work half-excluded, which reads as a finding
    # against someone else's file.
    "settings/src/lib/stores/models.ts",
)

# A store that shows fixture content on a real failure for a reason someone
# stands behind. Empty is the goal: the reason has to survive being read next to
# the sentence the user ends up seeing.
#
# The keys are `file:line`, which DRIFTS: editing anything above an entry moves
# it and the check reports it as new. That happened on 9 August, when making the
# meeting edits honest pushed one acknowledgement down twenty lines. Annoying,
# and the alternative is worse - keying on the file alone would hide a genuinely
# new fixture in a file that already has an acknowledged one, which is exactly
# how the sentinel store hid three switches behind one fixed function.
ACKNOWLEDGED: dict[str, str] = {
    # The three below are one decision. Before 9 August the viewers app fell onto
    # this branch in a real shell too: `initial_file` returning null hit a bare
    # `return`, leaving the window on its default render, which is the mock - a
    # shipped viewer opened on nothing showed a song called "Nightswim" with a
    # waveform and a playhead, uncaveated. A real session with no file now says
    # "No file is open." and this branch is what remains: the browser, where there
    # is no host to ask, and the `?demo=` path the screenshot harness drives. Both
    # are contexts where a sample IS the answer. If the `noFile` branch above it
    # ever goes, these three go back to being the defect they were.
    "apps/viewers/src/routes/+page.svelte:98": (
        "The demo face, reachable only with no Tauri host or an explicit `?demo=`; the real shell with no file is answered above it."
    ),
    "apps/viewers/src/routes/+page.svelte:100": (
        "The demo face, reachable only with no Tauri host or an explicit `?demo=`; the real shell with no file is answered above it."
    ),
    "apps/viewers/src/routes/+page.svelte:102": (
        "The demo face, reachable only with no Tauri host or an explicit `?demo=`; the real shell with no file is answered above it."
    ),
    'apps/text-editor/src/lib/stores/lens.ts:82': (
        "Caveat at the claim, and nothing here turns invented data into an argument. That is the line tonight's fixes drew: a labelled sample on screen is a design choice someone made, but a fixture that supplies an id, an index or a pid to a real call is a defect whatever the label says. The lens shows provenance, backlinks and project context for the open file, labelled 'Example context - not this file's real graph neighbourhood'. `openRelated` navigates rather than mutates."
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


def markup_fixtures(text: str):
    """Yield (line, name) for each fixture-named identifier rendered in markup.

    Markup is everything after the last `</script>`, minus comments and the
    `<style>` block - a CSS comment reading "Mocks the real tile strip" is prose
    about a skeleton, not a component rendering invented data, and it was the
    only false positive this pass produced across every app.
    """
    cut = text.rfind("</script>")
    if cut < 0:
        return
    offset = cut
    markup = text[cut:]
    # Blanked rather than removed, so the line numbers still point at the file.
    markup = SVELTE_COMMENT.sub(lambda m: "\n" * m.group(0).count("\n"), markup)
    markup = SVELTE_STYLE.sub(lambda m: "\n" * m.group(0).count("\n"), markup)
    for m in MARKUP_FIXTURE.finditer(markup):
        line = text[:offset].count("\n") + markup[: m.start()].count("\n") + 1
        yield line, m.group(0)


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
    rendered = 0
    used: set[str] = set()
    for path in sorted((ROOT / "apps").rglob("*.ts")) + sorted((ROOT / "apps").rglob("*.svelte")):
        s = str(path)
        if any(k in s for k in SKIP) or "node_modules" in s or "/src/" not in s:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        rel_path = path.relative_to(ROOT)
        if path.suffix == ".svelte":
            for line, name in markup_fixtures(text):
                rendered += 1
                if f"{rel_path}:{line}" in ACKNOWLEDGED:
                    used.add(f"{rel_path}:{line}")
                    continue
                findings.append(
                    f"{rel_path}:{line}: renders `{name}` straight into markup, so this "
                    f"component's resting state is invented content. No catch is "
                    f"involved and no store is written, which is why the pass above "
                    f"cannot see it."
                )
        if "catch" not in text:
            continue
        files += 1
        for line, body in catch_bodies(text):
            catches += 1
            if DEV_GUARD.search(body):
                continue  # the dev-only fallback this is asking for
            hit = None
            if STORE_WRITE.search(body) or RETURNS.search(body):
                hit = FIXTURE_NAME.search(body)
            hit = hit or MOCKED_FLAG.search(body)
            if not hit:
                continue
            rel = path.relative_to(ROOT)
            if f"{rel}:{line}" in ACKNOWLEDGED:
                used.add(f"{rel}:{line}")
                continue
            findings.append(
                f"{rel}:{line}: a failed read falls back to `{hit.group(0).strip()}`, so "
                f"invented content renders as fact. Guard it with "
                f"`import.meta.env.DEV` and give the real session an error the "
                f"surface can show."
            )

    print(
        f"{catches} catch block(s) across {files} frontend file(s) checked for a "
        f"fixture shown after a failed read, and {rendered} fixture-named "
        f"identifier(s) reaching markup. Named fixtures only: an empty list "
        f"in a catch can be the honest answer for some stores and a false claim "
        f"for others, which needs the store read rather than this."
    )
    # An acknowledgement that matched nothing. Either the fixture it excused is
    # gone, in which case the excuse must go with it, or the line drifted and the
    # real one is being reported above as new - and both halves want saying. Added
    # 9 August: making the meetings fixtures honest left four of these behind, and
    # nothing here noticed. The same rule was applied to `check-invoke-exists` a
    # few hours earlier; an exception must not outlive its subject.
    for key in sorted(set(ACKNOWLEDGED) - used):
        # Only where the file is actually in the tree being scanned. Without this
        # the guard fired on every entry the moment the check was pointed at a
        # throwaway tree, which its own test caught immediately: an acknowledgement
        # for a file that is not there says nothing about whether its subject
        # survived.
        if not (ROOT / key.rsplit(":", 1)[0]).exists():
            continue
        findings.append(
            f"{key}: acknowledged, but nothing at that line is a fixture fallback "
            f"any more. Delete the entry, or if the line moved, re-key it to where "
            f"the fixture went."
        )

    if findings:
        print("\nstores that answer a failed read with invented content:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
