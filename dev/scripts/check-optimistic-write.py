# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a mutation which failed does not leave the surface claiming it worked.

The shape, found four times in two hours on the night of 8 August:

    store.update(...)                 // show it immediately, it will succeed
    try { await invoke("do_the_thing") }
    catch { /* no backend yet: the optimistic state stands */ }

Written while the backend did not exist, when it is plainly right. Once the app
ships it means something else: the user pressed a button, the screen says it
happened, and it did not. Unlike a fixture, which is at least visibly generic,
this is the user's own action reflected back at them.

What it cost before it was named:

  * the timeline's pause switch said "Recording is paused. Nothing is added until
    you resume." while the daemon kept recording - the command has no host at all,
    so it was every session, not an edge case
  * revoking a capsule removed it from the list, telling someone a shared slice of
    their graph could no longer be read while it still could
  * minting one added a row for a capsule that was never minted
  * a Settings write kept the typed value on the row; the file still held the old
    one, so the next read snapped back and the change looked self-undoing

What this looks for: a store write (`.set` / `.update`) shortly BEFORE a `try`
whose body invokes, followed by a catch that does nothing at all - no revert, no
flag, no rethrow. All three parts, because each alone is fine: an optimistic write
that is reverted is correct, and a silent catch around a read that leaves an empty
state can be honest.

The fix that satisfies it is the one those four got: keep the optimism under
`import.meta.env.DEV`, and in a real session put the state back and say so where
the claim was made. "Say so" is not checkable here - a flag set and never rendered
would pass - so this is a floor, not a proof.

What it does NOT cover:

  * whether the revert is CORRECT (it cannot know what the previous value was)
  * a mutation whose optimistic write happens in the component rather than the
    store, or more than ~500 characters before the try
  * the write-AFTER-catch variant, which is the same defect with the lines
    swapped: `try { invoke } catch { }` and then an unconditional `store.update`
    below it. The printer page was four of those and this check could not see
    them - they were found by reading, after the gate pointed at the file for a
    different reason
  * a catch that logs and nothing else, which is silent to the user even though it
    is not empty in the source
  * `apps/harness` and `apps/store`, arlen-ui's live work

Shown to fail before being trusted: written against the 31 that were there, it
immediately named a 32nd I had missed by hand - `setAlerts` in the sentinel store,
one function below the `setDetector` I had just fixed. Reverting any of the fixes
puts that one back in the list.

REPAIRS, and which one depends on what actually failed. There are two, and the
wrong one does damage:

  1. REVERT, when the surface claims a change that did not happen. A timeline
     showing recording paused while it runs, a capsule gone from a list that was
     never revoked - the state is simply false, and putting it back is the fix.

  2. SAY WHICH HALF FAILED, when the action was compound and only its durable
     half failed. The greeter's screen-reader toggle applies the reader to THIS
     login (a store update, which always succeeds) and remembers it for the next
     start (a file write, which can fail). Reverting there switches off a screen
     reader somebody just asked for because a write failed - the exact harm this
     rule exists to prevent. The honest repair is a sentence beside the switch:
     applies now, will not be remembered.

This check cannot tell those apart. It sees an optimistic update and a swallowed
rejection, which both shapes have, so it names the finding and leaves the choice
to whoever knows what the action promised. Reverting on reflex is how a check
meant to stop a lie ends up telling one.
"""

import re
import sys
from pathlib import Path

# The tree to scan. Takes an argument so the check can be pointed at a fixture and
# SHOWN TO FAIL - it had no companion test for exactly this reason: a hardcoded
# root cannot be handed a planted violation, so the one rule this directory holds
# above the others ("a checker is not trusted until it has been shown to fail")
# was unmet here because the check could not be run against anything else.
ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

CATCH = re.compile(r"\}\s*catch\b[^{]*\{", re.S)
STORE_WRITE = re.compile(r"\.(set|update)\(")
#: The same optimistic update written as a Svelte 5 rune rather than a store: a
#: component reassigning its own state from itself, `cfg = { ...cfg, enabled }`.
#:
#: This gate's summary said for its whole life that it "cannot see a component-
#: level optimistic write", and that sentence was load-bearing: `STORE_WRITE`
#: matched `.set(`/`.update(` only, so every rune-based surface was outside it.
#: On 16 August the night-light section turned out to hold six of them - flip the
#: switch with the daemon down and it stayed flipped while nothing turned on,
#: which is the exact defect this file exists to prevent, in the one shape it
#: could not look at. Found by clicking the switch, not by reading.
RUNE_WRITE = re.compile(r"\b(\w+)\s*=\s*\{\s*\.\.\.\s*\1\b")


#: A function declared in this file whose own body writes a store. The clock keeps
#: one called `patch`, and every alarm and timer mutation goes through it, so the
#: two patterns above saw nothing in that entire app. That one happens to revert
#: correctly - but this check could not have said so either way, which is the
#: point: a gate matches the shape its author last happened to write.
LOCAL_WRITER = re.compile(
    r"function\s+(\w+)\s*\([^)]*\)[^{]*\{(?:[^{}]|\{[^{}]*\})*?\.(?:set|update)\(",
    re.S,
)


def local_writers(text: str) -> set[str]:
    """Names of same-file helpers that write a store when called."""
    return {m.group(1) for m in LOCAL_WRITER.finditer(text)}


def optimistic_update(code: str, writers: set[str] | None = None) -> bool:
    """Whether `code` sets a surface's state ahead of the call it depends on."""
    if STORE_WRITE.search(code) or RUNE_WRITE.search(code):
        return True
    return any(re.search(rf"\b{re.escape(w)}\s*\(", code) for w in (writers or ()))
COMMENT = re.compile(r"/\*.*?\*/", re.S)

# The blind spot, found by reading rather than by this check, on 9 August: the
# INVERSE shape, where the catch performs the mutation itself. The timeline's
# "delete this range for good" called the command, and on failure its catch
# dropped the range from the store - so a delete that never reached the graph
# cleared the screen and told the user their history was gone. This check cannot
# see it, because it looks for an EMPTY catch after an optimistic write and that
# catch was full.
#
# It is not mechanically separable either, and the measurement says so: 14 catch
# blocks in the tree mutate through `.update(` without a DEV gate, and the ones
# sampled are correct REVERTS or error-flag writes - `topbar.update(s => ({...s,
# error}))`, `keybindings` the same. A revert and a fake success are the same
# syntax; only the direction differs, and the direction is semantic. Flagging the
# shape would fire on the right code, which is how a check stops being read.
#
# So it is written down here rather than gated, and the way it gets found is a
# person reading a catch on a control that promises something irreversible.

SKIP = ("/harness/", "/store/", "node_modules")

# The ones that were already there, worst first. This is a queue, not an alibi:
# every line is a button that lies when its backend is down, and most of them have
# no backend at all (see check-invoke-exists.py). An entry leaves this list when
# the action is made honest, not when it is explained.
# File to (how many are there today, why). The COUNT is what keeps this a queue
# rather than a hole: a file-keyed exception hides every new instance added to an
# already-listed file, which is exactly what the opener check turned out to be
# doing when it was re-proved on 9 August. A file that grows a new one fails,
# and a file whose count drops asks to have its number lowered.
KNOWN: dict[str, tuple[int, str]] = {
    "apps/settings/src/lib/stores/models.ts": (
        3,
        "setRole, startDownload, cancelDownload - arlen-ui's model picker is live "
        "work; named rather than skipped because the shape is identical"
    ),
    "apps/settings/src/lib/stores/themes.ts": (
        1,
        "installThemeFile, where a cancelled file picker and a failed install "
        "arrive as the same exception - telling them apart needs an error shape "
        "the command does not return yet"
    ),
}

# Four entries left this list on 12 Aug without a line of app code changing, and
# that is worth reading rather than tidying away. `windows-apps.ts` (2),
# `appSettings.ts`, `waypointerAsk.ts` and `lens.ts` were all matched by a store
# write in the function ABOVE the one being checked, because the lookbehind was
# 500 raw characters rather than "within this function". Two of them had said so
# in their own reason text - "matched by the proximity window rather than by being
# wrong", "belongs to the streaming function above it, inside the 500-character
# window" - which is a gate's false positive being written down instead of fixed.
# `models.ts` dropped 4 to 3 the same way.
#
# A queue that carries a checker's own bugs stops being a queue: every entry reads
# as work somebody owes, and four of these six owed nothing.


def catch_spans(text: str):
    for m in CATCH.finditer(text):
        depth, i = 1, m.end()
        while i < len(text) and depth:
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
            i += 1
        yield m.start(), m.end(), i - 1


#: How much CODE to look back over for the store write that the `try` then risks.
#: Roughly the "same visual block" distance, which is what the rule means.
PRECEDING_CODE = 500


def preceding_code(head: str, upto: int) -> str:
    """The code shortly before `upto`, with comments removed.

    The window is over CODE, not raw characters, and that distinction is the whole
    reason this function exists. It was `head[upto - 500 : upto]` verbatim, so a
    long comment between the store write and the `try` pushed the write out of the
    window and the pair went unreported - a MISS, on a gate whose entire subject is
    a surface that claims something untrue. The catch body a few lines below has
    stripped comments since it was written; the lookbehind had not, and nothing
    made the inconsistency visible.

    Found on 12 Aug by the same defect one file over: a twelve-line comment above
    `.arg("--")` pushed it out of `check-opener-args`'s 600-character chain window,
    and that gate reported a call it had just been taught to accept. There the cost
    was a false finding; here it would have been silence.

    Then bounded by the enclosing function, which the character count had been
    standing in for by accident. Stripping comments alone made the window reach
    back PAST a function's closing brace into the one above it, and reported three
    calls whose `try` has no optimistic write anywhere near it - `installThemeFile`
    picked up `setTheme`'s write, four lines and one function boundary away. A
    wider window found "more", and all of it was wrong. The last top-level `}` is
    the real edge, and the raw count was only ever approximating it.
    """
    raw = head[max(0, upto - PRECEDING_CODE * 4) : upto]
    code = COMMENT.sub("", "\n".join(l.split("//")[0] for l in raw.splitlines()))
    # A closing brace in the first column ends the previous top-level item in this
    # codebase's style, so nothing before it belongs to this call's function.
    edge = code.rfind("\n}")
    if edge != -1:
        code = code[edge:]
    return code[-PRECEDING_CODE:]


#: `invoke(...)` whose rejection is handed to a `.catch(` handler, rather than
#: sitting inside a `try`. The promise form was invisible to this gate for its
#: whole life, and its own summary said so - "cannot see ... a catch that only
#: logs" - which is a limit stated honestly and still a limit. Four instances sat
#: behind it in the notification store, where the optimistic update is the one
#: that matters most: dismissing removes the row and clearing empties the panel.
PROMISE_CATCH = re.compile(r"\binvoke\s*(?:<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\s*\(")


def balanced(text: str, open_at: int) -> tuple[str, int]:
    """The text inside the bracket pair at `open_at`, and the index after it."""
    if open_at >= len(text) or text[open_at] not in "([{":
        return "", open_at
    depth, i = 0, open_at
    while i < len(text):
        if text[i] in "([{":
            depth += 1
        elif text[i] in ")]}":
            depth -= 1
            if depth == 0:
                return text[open_at + 1 : i], i + 1
        i += 1
    return "", len(text)


def only_reports_to_a_log(handler: str) -> bool:
    """Whether a catch handler does nothing a person could notice.

    `console.error(...)` is the shape that reads like handling and is not: the
    surface has already claimed the change happened, and the only correction goes
    somewhere nobody is looking. An empty handler counts too - it is the same
    thing with less ceremony.
    """
    code = COMMENT.sub("", "\n".join(l.split("//")[0] for l in handler.splitlines()))
    # Strip the arrow-function scaffolding, then every console call, and see
    # whether any statement is left.
    code = re.sub(r"^\s*\(?\s*\w*\s*\)?\s*=>\s*", "", code.strip())
    while True:
        m = re.search(r"console\s*\.\s*\w+\s*\(", code)
        if not m:
            break
        _, after = balanced(code, m.end() - 1)
        code = code[: m.start()] + code[after:]
    return not re.sub(r"[\s{};,]", "", code)


def blank_comments(text: str) -> str:
    """`text` with comment CONTENT replaced by spaces, offsets and lines intact.

    The promise-catch scan used to read the raw file, so a comment quoting the
    shape it looks for was a finding. That is not hypothetical: the fix to
    meetings' `stopCapture` documents what it replaced - `invoke(...).catch(() =>
    {})` - and the gate reported the sentence describing the repair as the defect.
    A checker that cannot tell code from prose about code teaches people not to
    write the prose.
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)


def promise_catch_findings(text: str, rel: str):
    """Optimistic store write, then `invoke(...).catch(<only logs>)`."""
    out = []
    writers = local_writers(text)
    text = blank_comments(text)
    for m in PROMISE_CATCH.finditer(text):
        _, after = balanced(text, m.end() - 1)
        tail = text[after : after + 12]
        cm = re.match(r"\s*\.\s*catch\s*\(", tail)
        if not cm:
            continue
        handler_at = after + cm.end() - 1
        handler, _ = balanced(text, handler_at)
        if not only_reports_to_a_log(handler):
            continue
        if not optimistic_update(preceding_code(text[: m.start()], m.start()), writers):
            continue
        out.append(text[: m.start()].count("\n") + 1)
    return out


def main() -> int:
    findings: list[str] = []
    known_hits = 0
    checked = 0
    # How many instances each file has produced so far this run, so an entry's
    # recorded count bounds it rather than excusing the file forever.
    seen_per_file: dict[str, int] = {}
    # `daemons/*/…/src` too: the file picker writes through `invoke` like any
    # frontend, and a picker that says a file was chosen when the daemon refused
    # is the same lie in the dialog every app borrows.
    trees = [ROOT / "apps", ROOT / "daemons"]
    files = sorted(f for t in trees for f in t.rglob("*.ts"))
    files += sorted(f for t in trees for f in t.rglob("*.svelte"))
    candidates = 0
    for path in files:
        s = str(path)
        if any(k in s for k in SKIP) or "/src/" not in s:
            continue
        candidates += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        if "invoke(" not in text:
            continue
        rel = str(path.relative_to(ROOT))
        writers = local_writers(text)
        for line in promise_catch_findings(text, rel):
            checked += 1
            seen_per_file[rel] = seen_per_file.get(rel, 0) + 1
            if rel in KNOWN and seen_per_file[rel] <= KNOWN[rel][0]:
                known_hits += 1
                continue
            findings.append(
                f"{rel}:{line}: a store was updated optimistically and the call's "
                f"rejection goes to `console`, so the surface states that something "
                f"happened which did not and the only correction is somewhere "
                f"nobody is looking. Say so where the claim was made - and read "
                f"REPAIRS in this file's header before reverting."
            )
        for start, body_start, end in catch_spans(text):
            checked += 1
            body = COMMENT.sub("", "\n".join(l.split("//")[0] for l in text[body_start:end].splitlines()))
            if body.strip():
                continue
            head = text[:start]
            try_at = head.rfind("try")
            if try_at < 0 or "invoke(" not in text[try_at:start]:
                continue
            if not optimistic_update(preceding_code(head, try_at), writers):
                continue
            seen_per_file[rel] = seen_per_file.get(rel, 0) + 1
            if rel in KNOWN and seen_per_file[rel] <= KNOWN[rel][0]:
                known_hits += 1
                continue
            line = head.count("\n") + 1
            findings.append(
                f"{rel}:{line}: a store was updated optimistically, the call failed, "
                f"and the catch does nothing - so the surface states that something "
                f"happened which did not. Keep the optimism under "
                f"`import.meta.env.DEV` and say so where the claim is made - and "
                f"read REPAIRS in this file's header before reverting."
            )

    # The half this file has claimed since it was written - "a file whose count
    # drops asks to have its number lowered" - and never implemented. It matters
    # the moment the gate gets more precise: tightening the lookbehind to stop at
    # a function boundary on 12 Aug took four of these six entries to zero, and
    # without this they would have sat there describing instances that no longer
    # exist. Two of them had even written the gate's own bug into their reason
    # ("matched by the proximity window rather than by being wrong"), which is a
    # false positive being documented instead of fixed.
    # Scoped to this tree, for the reason `check-peer-identity-sandbox.py` is:
    # KNOWN is a set of counts about ONE repo, and a fixture lacks those files for
    # reasons that have nothing to do with the entries being stale. Without this
    # every fixture case reports both real entries as dropped. The check proper
    # still runs against any tree it is handed; only this self-audit cannot.
    audits_own_list = len(sys.argv) <= 1 or ROOT == Path(__file__).resolve().parents[2]
    for rel, (declared, _) in sorted(KNOWN.items() if audits_own_list else []):
        found = seen_per_file.get(rel, 0)
        if found < declared:
            findings.append(
                f"{rel}: carried as {declared} known instance(s) and only {found} "
                f"remain. Lower the number, or drop the entry if it is zero - a "
                f"count that is too high reserves room for a new one to appear "
                f"unreported."
            )

    # Zero CATCH BLOCKS is a legitimate answer: a frontend can be written without
    # one. Zero candidate FILES is not - it means no app source was found where
    # every app keeps it, so the scan ran against the wrong root or the layout
    # moved, and the sentence below would describe a tree this never opened.
    if not candidates:
        print(
            f"NOTHING WAS READ: no .ts or .svelte under {ROOT / 'apps'}/*/src",
            file=sys.stderr,
        )
        return 2

    print(
        f"{checked} catch block(s) checked for a mutation that failed silently after "
        f"an optimistic update. {known_hits} in {len(KNOWN)} known file(s) carried as "
        f"a queue, each bounded by its recorded count. A rejection handed to "
        f"`.catch` is read, log-only handlers included, and a component setting "
        f"its own state (`x = {{ ...x }}`) counts as the optimistic update - it "
        f"did not until 16 August, and six night-light setters lived in that gap. "
        f"Still cannot see a revert's correctness, or an update written some "
        f"third way."
    )
    if findings:
        print("\nactions that claim to have happened when they did not:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
