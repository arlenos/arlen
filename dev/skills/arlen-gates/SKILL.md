---
name: arlen-gates
description: How a mechanical check earns its place in an Arlen repo - what makes one worth writing, the control that proves it can still fail, the false-positive traps that get checks switched off, and how to wire it so it actually runs. Invoke BEFORE writing any check, lint, gate or CI step, and when a defect you just fixed looks like it could recur.
---

# A check nobody can see fail is an assertion, not a check

There are about a hundred and sixty of them in `~/Repositories/arlen/dev/scripts` - 161 counted on 10
September, and the count is written loosely on purpose: it said 151 for long enough to be wrong, and a number
a reader half-trusts is worse than a range they can check in one command
(`ls dev/scripts/check-*.{py,sh,mjs} | wc -l`). The ones that earn their place share a shape, and the ones
that got switched off shared a different one. This is that shape, learned by getting it wrong.

## When a check is worth writing

**After a real defect, and named after what the defect was.** Every good one here started as something that had
already happened: 57 `invoke` calls whose command no host registered; a store answering a failed read with
invented printers; a comment pointing at a file that had moved. Writing a check for a defect you imagine
produces a check nobody trusts, because the first time it fires nobody believes it.

**Not when the population is a judgement call.** Two attempts here to check "does this control assert something
GOOD" both failed, because the spellings are not uniform and the question is a person's. A check that reports
correct code gets disabled within a week, and then it protects nothing. Flag the mechanically absent thing;
leave the quality to a reader, and say in the docstring that you did.

**Green on arrival is fine, and often the right moment.** A check whose current population is zero locks in a
property that is true today. Say so in its output.

## Measure the population BEFORE you believe your pattern

This is the step that separates the checks that work from the ones that get reverted.

- A first pattern for "is a repo path in this comment" matched 65 things, essentially all prose (`dev/null`,
  `apps/AI`). Requiring a file extension took it to 7 candidates, 4 of them real.
- A first pattern for "does this control assert" read `ok(` and `bad(` and reported 60 of 156, because half of
  them name the helper `check()`. The real oracle was not a word at all - a control's verdict is its exit code.
- A scouting script once passed a whole corpus for the wrong reason (its pattern happened to include
  `process.exit(`), and trusting it would have shipped a check believing the tree clean.

**Run the pattern over the tree and read the output before writing the gate.** If it reports things you would
not act on, the pattern is wrong, not the tree.

## The control is the point

`check-controls-exist.py` refuses a `check-*` with no `test-check-*` beside it, and that is structural rather
than a matter of remembering - the moment to skip the control is exactly the moment a new check feels obviously
right.

A control mints a throwaway tree, plants the defect, runs the gate against that tree, and asserts it goes red.
Then plants the correct version and asserts it goes green. **Both directions**, or you have not shown the gate
discriminates.

- Take the fixture helper (`dev/scripts/lib/fixture.mjs`) - the gates run concurrently, and a control that edits
  the real tree turns three unrelated checks red.
- **The best control case is the defect that actually happened.** Checking two files out at the commit before
  their fix and watching the gate name the exact line is as strong as it gets.
- Include the cases that would make it useless. A pattern insisting on `{$name}` would have missed all 114
  `{$n :number}` placeholders; a landmark check that fired on another lane's surface would have been switched off.

## Say what you did NOT look at

Every gate here ends by printing what it read and what it cannot say. That line is load-bearing: "0 violations"
from a sweep that swept nothing looks identical to a clean run. A gate that reads zero files should exit 2 with
`NOTHING WAS READ`, not 0 - a wrong root is the likeliest way to get a confident green.

## Deriving a list beats maintaining one, and a filter is a maintained list wearing a costume

The gate runner reads CI's list out of the workflow rather than keeping its own copy. It still had a hole: it
matched `dev/scripts/*.{py,mjs,sh}`, which is a filter on where a gate LIVES, so two gates that are cargo
binaries were invisible and a commit landed red on them twice in two days. **If your derivation has a shape
requirement, that requirement is the next hole.**

## KNOWN, with a reason, not a baseline of names

When a real exception exists, name it in the script with the reason it is legitimate, one line, saying what
would break if it were "fixed". A bare allowlist rots into a place people add names to make a red go away.

## Wiring, or it does not run

1. The check in `dev/scripts/check-<thing>.py` (or `.mjs`).
2. Its control in `dev/scripts/test-check-<thing>.mjs`.
3. Both added to the structural-checks step in `.github/workflows/ci.yml`.

The pre-commit hook derives its list from that workflow, so step 3 is what makes it run before a commit rather
than after. `check-wired.py` refuses a check that is in the tree and in no run list; `check-tests-run.py` does
the same for tests. Expect one of them to refuse your first commit - that is them working.

## The recurring lesson

**A gate matches the shape its author last happened to write.** Three of these were widened in one week by
finding the thing they were blind to: a lint that read a prose literal assigned to a name but not one passed to
a function; a duplicate check that looked inside one file while the catalogue was split across two; a stale-path
check whose prefix list was the directories that exist NOW, so every reference to the layout before a
restructure passed. When a defect gets through a gate that should have caught it, the gate is the finding.
