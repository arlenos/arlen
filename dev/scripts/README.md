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

Twenty-nine of the forty-one checks have one, and **nothing enforces it** - which
this file said otherwise until it was checked. That gap is a decision rather than
an oversight: writing the missing twelve as a batch is an evening on test
scaffolding, so they are written when their subject is next touched, while the
defect is fresh and the fixture is obvious. The count is here so the gap is a
number somebody can watch rather than a feeling.

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

## Where the lists live

Some checks are held to a declaration rather than to a scan, because deriving the
list would be worse than keeping it:

- `served-objects.tsv` - the bus surfaces and the object path each must serve.
  `check-bus-names-covered.py` requires every `BusName=` in a shipped unit to
  appear here, so the list cannot quietly shrink.
- `gated-methods.tsv` - what each D-Bus method must do when a caller that
  resolves to no app id calls it: `refuse`, or `open:<reason>`.

An entry excused from a rule carries the reason inline, and the reason is read by
a person. `check-wired.py` also checks its own exemption list in both directions:
an excuse for something that is now run, or for something that no longer exists,
is itself reported.
