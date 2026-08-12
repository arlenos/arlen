# dev/scripts

Two kinds of thing live here, plus the install helpers this directory started as.

**Checks** (`check-*`) read the tree and exit non-zero when something is wrong.
They run in CI and in the pre-commit sweep. **Probes** (`probe-*`) drive running
software - a daemon on a private bus, an app under Xvfb - and answer questions
source cannot: whether an object is really served, whether a gate really refuses.
Probes need built binaries and seconds of wall clock, so they are invoked by a
`just` recipe rather than by CI.

Each check carries its own reasoning in its docstring: what it looks for, what it
does NOT cover, and the defect that produced it. That is the format to follow,
and it is worth the space - the exemptions and the boundaries are where these
files go wrong, and a rule whose reason is written down can be argued with.

## What a new check has to do

These four are not style. Each one is a way a check has passed while measuring
nothing, and each was found the hard way.

**Take the tree as an argument on day one.** `ROOT = sys.argv[1] if ... else
<repo>`. A check that can only run against the tree it was written for cannot be
shown to fail, and thirteen of these had no positive control for exactly that
reason - untestable and untested arrived together.

**Be shown to fail before you trust it.** A `test-check-*.mjs` beside it, running
the check against a planted defect and against the correct form.

**Nothing enforces this** - which this file claimed otherwise until it was
checked. The gap is a decision rather than an oversight: writing the missing ones
as a batch is an evening on test scaffolding, so they are written when their
subject is next touched, while the defect is fresh and the fixture is obvious.

`check-wired.py` prints how many checks have a control and names the ones that do
not, every run. It is not a number in this file on purpose: two hand-counts of it
tonight were both wrong - once by counting test FILES instead of proven checks,
once by matching names instead of reading which check each control actually
drives. A count belongs in the tool that can recompute it.

**Compare your count to the size of what you claim to cover.** Print how much was
scanned, not only how much was flagged. `check-opener-args.py` answered "pass" to
an empty directory for its whole life: zero calls checked, zero problems, green.
A count of zero is only honest when there was something to count.

**Make the unmatched case the loud one.** A classifier that scores by string match
and defaults to "fine" is always one wording behind whoever writes the text it
reads. `probe-dbus-gate.sh` scored four different phrasings of "the bus rejected
this before your gate ran" as ANSWERED - reporting a method as ungated when the
gate was never consulted. Unknown must mean "nothing was proven" and fail.

The same shape has one more disguise worth naming: **an assertion loose enough to
be satisfied by an unrelated event**. `stderr.contains("too large") ||
contains("decode")` passed with the size guard deleted, because the decode failed
anyway. If a test can pass for a reason other than the one it is named after, it
is measuring that other reason.

**Bound a scan by structure, never by a character count.** Every windowed scan
here has been wrong at least once. `check-opener-args` looked 600 raw characters
down a builder chain, and a twelve-line comment above `.arg("--")` pushed the
argument out of the window - the gate reported a call it had just been taught to
accept, defeated by prose. `check-optimistic-write` looked 500 raw characters back
for the store write its `try` risks, so a comment could hide a real one. Stripping
comments fixes the obvious half and exposes the subtle half: with the padding gone
the window reached PAST a function's closing brace and reported three calls whose
`try` has no optimistic write at all. The count had been standing in for "within
this function" by accident. `check-read-grants-cover-queries` learned it first and
writes it best - a window that overshoots "does not look wrong, it looks like a
finding, and the fix it invites is granting a field nothing reads". Anchor on the
brace, the string literal, the function edge. If you must count, count code.

**A uniform finding is a caller you cannot see, not many independent mistakes.**
Twice in one night a scanner produced a confident wrong answer with the same
tell. Thirteen apps "over-granting" close and minimize - the calls were in the
ui-kit's `WindowControls`, which they all render. Four apps granting a plugin
command they demonstrably invoke - those calls come from other kit modules. Real
defects cluster and vary; a result that is identical across every subject usually
means the evidence lives somewhere the scan did not look. Before reporting one,
find the shared thing. And note that widening a scan RE-OPENS this question: the
window family had its shared caller accounted for, and extending the same check to
plugin permissions walked straight back into it, because the credit was written
for one component.

**Do not measure the label when you can read the thing.** `check-wired` counts
positive controls by reading which check each test file DRIVES, after two
hand-counts got it wrong by matching names. The same mistake surfaced twice more
in one night, both mine: an exemption-list survey that matched list NAMES missed
every list somebody had named sensibly, and a cross-reference survey matched the
inner substring of `test-check-fixtures.mjs` and reported thirteen files as
missing. A survey is a measurement and deserves the same suspicion as a check.

## Where the lists live

Some checks are held to a declaration rather than to a scan, because deriving the
list would be worse than keeping it:

- `served-objects.tsv` - the bus surfaces and the object path each must serve.
  `check-bus-names-covered.py` requires every `BusName=` in a shipped unit to
  appear here, so the list cannot quietly shrink.
- `gated-methods.tsv` - what each D-Bus method must do when a caller that
  resolves to no app id calls it: `refuse`, or `open:<reason>`.

An entry excused from a rule carries the reason inline, and the reason is read by
a person. Three things go wrong with those lists, all of them found on 12 Aug by
looking, and each list here now guards against the ones that apply to it.

**An excuse outlives its subject.** `check-wired.py` had checked its own list in
both directions from the start - an excuse for something now run, or for something
that no longer exists, is itself reported - and nothing else did. Four lists have
that guard now, and adding it to `check-invoke-scope` immediately found two
acknowledged cross-app calls that had both been FIXED, each app having grown its
own command of that name, with the entries still describing them as broken.

**An excuse names the wrong owner**, which is worse than a stale one because
nothing is watching the file at all: the gate stops looking and the named owner
never learns they were named. `check-knowledge-socket` excused two files as
arlen-ui's live work; both were under `src-tauri`, which is not their lane, and
both were resolving the graph socket from the daemon's BIND variable - dead on a
booted image. Check the boundary before you write the name into a list.

**An excuse describes the CHECKER rather than the tree.** The clearest tell is an
entry that explains the gate's own mechanism: `check-optimistic-write` carried one
reading "matched by the proximity window rather than by being wrong" and another
"the store write this check sees belongs to the streaming function above it". Both
diagnosed a false positive precisely and filed it as work instead of fixing the
window. Four of that list's six entries turned out to be the checker's bug. If you
find yourself writing the gate's internals into a reason, fix the gate.

A count beside each entry, where the subject can grow, is what keeps a list a
queue rather than a hole - and the count has to be checked in BOTH directions.
`check-optimistic-write` promised in its own docstring that "a file whose count
drops asks to have its number lowered" and never implemented it, which is how four
retired entries would have sat there unnoticed.
