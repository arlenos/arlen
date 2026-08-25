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
    Down to 3 on 25 August, and all three were read rather than left as a number:
    `nowPlaying.set(null)` is argued in place (an absent media player is not a
    claim); files' `savedSearches.set([])` makes the sidebar group VANISH rather
    than say "none saved", so nothing false is stated; calendar's
    `calendars.set([])` is the same, and its comment claimed a different fallback
    than the one it sits on, which is fixed. The queue is empty of defects, so
    what remains here is the rule and not a backlog.
  * A fixture reached through a helper, or assigned to a local that is set
    later.
  * Invented content written INLINE in the catch - `store.set([{ id: 1, label:
    "Wake up" }])` passes where `store.set(FIXTURE_ALARMS)` fails, measured on
    11 August. This one is deliberate and not a widening waiting to happen: an
    inline literal in a catch is just as often the HONEST fix (`{ status:
    "unavailable" }`, an error flag the surface renders), so flagging the shape
    would punish the correction this check exists to encourage. Telling invented
    content from an honest failure state is a reading, not a regex.
  * Whether a store that DOES branch on `isTauri()` or `import.meta.env.DEV` put
    the fixture on the right side of it.
  * A catch that returns and leaves a fixture ALREADY on screen. The `return`
    half above only fires when a fixture name sits in the catch; when the
    fixture arrives by falling through to the default render, the catch body is
    a bare `return` and there is nothing here to match. That is how the viewers
    app kept its "Nightswim" branch through the 9 August fix and until 16
    August - the catch beside the one that was fixed said only `return`, and the
    mock was three hundred lines away in the markup.

    Not widened to "a bare return in a file whose fixture reaches markup",
    though that rule would have caught it: viewers has many catches and one of
    them mattered, so it would demand acknowledgements for the rest and teach
    people to write them without reading. What actually finds this one is
    shooting the app with no backend and looking, which is what
    `shoot-no-backend.sh` is for.
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

It passes today, and this paragraph used to say the opposite: that it "fails on
one store, `models.ts`, left visible rather than excused". Both halves went stale
without anyone noticing - models.ts is in the SKIP list forty lines below, so it
is excused rather than visible, and the check has exited clean since the meetings
fixtures were made honest. A file whose whole job is catching claims that outlived
their subject had two of its own; corrected 9 August. Five entries are
acknowledged with reasons, and every one of them names the condition that would
end it.

The line the acknowledgements draw, after fixing eleven of these in one night:
a labelled sample on screen is a design choice, and someone made it deliberately;
a fixture that supplies an id, an index or a pid to a real call is a defect
whatever the label says. The system monitor was the clearest case - its rows are
labelled 'Example processes' and their ids are 1, 101, 102, 103, which is what
the Stop button passes to the backend. That is the point of committing it - the four
found by reading were a sample and not the set, and a list in the repo is worth
more than the same evening repeated four more times. It has been in CI since
9 August, so the queue it once held is now a gate: the list reached empty, and
what is left in ACKNOWLEDGED is five deliberate samples rather than a backlog.
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
#
# A THIRD time, 20 August, and this one had teeth. Every spelling above expects
# the fixture word at the START of the identifier, so `buildFixture()` matched
# nothing: not `FIXTURE` (wrong case), not `\bfixture\w*\(` (no word boundary
# between `build` and `Fixture`). The screenshot tool called exactly that on the
# branch where a host could not capture the screen, and drew an invented desktop
# with a made-up account and a made-up token, uncaveated, with Save enabled and
# the thumbnail auto-saving it. It was found by opening the app, not by this.
#
# The embedded case is `Fixture` and `Mock` only. `demo` and `sample` start real
# English words, and writing the control for this turned up a false positive that
# had been here from the start: `demonstrate(` in a catch matched `demo\w*\(` and
# was reported as a fixture. So those two now need a non-lowercase char after
# them - `demo(`, `demoData(`, `DEMO_ROWS` still match, `demonstrate(` no longer
# does. `sampleRate(` would still match and there is none in the tree today; it
# is a name worth an acknowledgement rather than a looser pattern.
FIXTURE_NAME = re.compile(
    r"\b(FIXTURE\w*|\w*_FIXTURE|MOCK\w*|DEMO\w*|SAMPLE\w*)\b"
    r"|\b(fixture\w*|mock\w*|demo(?![a-z])\w*|sample(?![a-z])\w*)\s*\("
    r"|\b\w+(Fixture|Mock)\w*\s*\("
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
# Any of these counts as the author having separated the two sessions. Whether
# they got the branches the right way round is not something a regex settles;
# what it can see is that the question was asked at all.
#
# `tauriAvailable` is the one to reach for. This check used to name only
# `import.meta.env.DEV`, and the tree followed it into a real defect: a DEV build
# under `tauri dev` HAS a backend, so a command that genuinely failed there took
# the fixture branch and the page showed invented printers, capsules and grants;
# a release build rendered headlessly is not a DEV build and has no backend, so
# it took the other branch and reported a failure that had not happened. Asking
# for `__TAURI_INTERNALS__` answers the question the branch is really about.
# `check-host-vs-devbuild.py` holds that line going forward.
DEV_GUARD = re.compile(r"import\.meta\.env\.DEV|isTauri\(|tauriAvailable")

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
# The keys are `file:FIXTURE-NAME`, and the name is what the check already prints
# in its own finding - `FIXTURES`, `audioMock`.
#
# It used to be `file:line`, which DRIFTS: editing anything above an entry moves
# it and the check reports it as new. That cost a re-key on 9 August and again on
# 14 August, when threading a flag through `provenance.ts` pushed its entry down
# fifty lines and refused a commit that had nothing to do with fixtures. A gate
# that goes red because a file moved is the kind people learn to wave through.
#
# Keying on the FILE alone was rejected for a good reason and still is: it would
# hide a genuinely new fixture in a file that already has an acknowledged one,
# which is how the sentinel store hid three switches behind one fixed function.
# The name keeps that apart - the viewers app's three entries below are three
# different names in one file, and each still stands on its own.
#
# The one thing a name key collapses is the SAME fixture rendered twice in one
# file. That is one decision wearing two hats, so one acknowledgement is the
# honest granularity for it.
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
    #
    # There were TWO routes in, and 9 August closed one. The `catch` beside that
    # null check returned to the same mock, and it sits after the `tauriAvailable`
    # guard - `__TAURI_INTERNALS__` is present from the moment a real webview
    # loads - so a shipped viewer whose backend threw on `initial_file` landed on
    # "Nightswim" too. Closed 16 August; the catch now sets `loadError`. The
    # acknowledgement below said "the real shell with no file is answered above
    # it" for a week while one branch over still answered with a song. Worth
    # remembering when reading any entry here: what is acknowledged is the path
    # the author had in mind, and the code may have more of them than the note.
    "apps/viewers/src/routes/+page.svelte:audioMock": (
        "The demo face, reachable only with no Tauri host or an explicit `?demo=`; the real shell with no file is answered above it."
    ),
    "apps/viewers/src/routes/+page.svelte:imageMock": (
        "The demo face, reachable only with no Tauri host or an explicit `?demo=`; the real shell with no file is answered above it."
    ),
    "apps/viewers/src/routes/+page.svelte:videoMock": (
        "The demo face, reachable only with no Tauri host or an explicit `?demo=`; the real shell with no file is answered above it."
    ),
    'apps/text-editor/src/lib/stores/lens.ts:FIXTURE': (
        "Caveat at the claim, and nothing here turns invented data into an argument. That is the line tonight's fixes drew: a labelled sample on screen is a design choice someone made, but a fixture that supplies an id, an index or a pid to a real call is a defect whatever the label says. The lens shows provenance, backlinks and project context for the open file, labelled 'Example context - not this file's real graph neighbourhood'. `openRelated` navigates rather than mutates."
    ),
    "apps/files/src/lib/stores/provenance.ts:FIXTURES": (
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
    "apps/screenshot/src/routes/+page.svelte:isSample": (
        "This entry exists because of what it looked like an hour before it was "
        "written, which is the argument for opening apps rather than reading them. "
        "The tool asked the host for the screen and answered BOTH 'there is no host' "
        "and 'a host that cannot capture' with the same invented desktop: a card "
        "reading 'Signed in as tim@example.com / token: sk-9f2c1a7b4e88', uncaveated, "
        "with Copy and Save enabled beside it and the thumbnail's Dismiss auto-saving "
        "it to the screenshots directory unasked. Not a rendering blemish - a picture "
        "of a machine that does not exist, written to disk and sent to somebody. Now "
        "`capturePrimary` returns three outcomes instead of two: a real host that "
        "cannot capture gets an empty surface naming the cause and no way to save "
        "anything, and this flag is what remains - the browser and the render "
        "harness, where there is no screen to photograph and a sample IS the answer. "
        "It is labelled at the claim ('This is an example picture, not your screen.') "
        "directly above the canvas it describes. Revisit if the fixture ever becomes "
        "reachable with a host attached, which is the branch that made this a defect."
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
    scanned = 0
    files = 0
    catches = 0
    rendered = 0
    used: set[str] = set()
    # The daemons carry frontends too - the file picker is the dialog every app
    # borrows - and a fixture shown there would be a listing nobody has.
    trees = [ROOT / "apps", ROOT / "daemons"]
    for path in sorted(f for t in trees for f in t.rglob("*.ts")) + sorted(
        f for t in trees for f in t.rglob("*.svelte")
    ):
        s = str(path)
        if any(k in s for k in SKIP) or "node_modules" in s or "/src/" not in s:
            continue
        scanned += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        rel_path = path.relative_to(ROOT)
        if path.suffix == ".svelte":
            for line, name in markup_fixtures(text):
                rendered += 1
                if f"{rel_path}:{name}" in ACKNOWLEDGED:
                    used.add(f"{rel_path}:{name}")
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
            name = hit.group(0).strip()
            if f"{rel}:{name}" in ACKNOWLEDGED:
                used.add(f"{rel}:{name}")
                continue
            findings.append(
                f"{rel}:{line}: a failed read falls back to `{name}`, so "
                f"invented content renders as fact. Guard it with "
                f"`tauriAvailable` and give the real session an error the "
                f"surface can show."
            )

    # Zero catches, zero rendered fixtures and zero `files` are all legitimate
    # answers - `files` counts only the ones containing a catch, and this gate's
    # own markup cases are components with none. What cannot be legitimate is
    # having OPENED nothing: this reads `apps/**/src` and nowhere else, so an empty
    # scan means the frontends were not found and every count below describes a
    # tree this never looked at. Guarding on `files` instead was tried first and
    # the markup cases caught it immediately.
    if not scanned:
        print(
            f"NOTHING WAS READ: no .ts or .svelte under {ROOT / 'apps'}/*/src",
            file=sys.stderr,
        )
        return 2

    print(
        f"{catches} catch block(s) across {files} frontend file(s) checked for a "
        f"fixture shown after a failed read, and {rendered} fixture-named "
        f"identifier(s) reaching markup. Named fixtures only: an empty list "
        f"in a catch can be the honest answer for some stores and a false claim "
        f"for others, which needs the store read rather than this."
    )
    # An acknowledgement that matched nothing: the fixture it excused is gone, or
    # it was renamed, and either way the excuse must not outlive its subject. With
    # a name key this no longer fires for a file that merely moved, which is what
    # it kept doing and what taught people to re-key rather than look. Added
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
            f"{key}: acknowledged, but no fixture by that name is a fallback there "
            f"any more. Delete the entry, or if it was renamed, re-key it to the "
            f"name the fixture goes by now."
        )

    if findings:
        print("\nstores that answer a failed read with invented content:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
