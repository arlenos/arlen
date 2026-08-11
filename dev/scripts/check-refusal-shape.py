# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a refused D-Bus caller gets an error, not an empty value.

This exists because the same mistake was written nine times in one day, in five
files, by people reasoning independently and each arriving at a local
justification that sounded fine:

    the wire is JSON the caller parses, so an empty array is kinder than an error
    a method whose result arrives on a signal has no error channel
    the warning is in the journal, so the refusal is not lost

What none of them could see from where they sat is the sentence a user ends up
reading. An empty recent-actions list says *you have done nothing* to someone who
has. An empty installed-apps list says *nothing is installed*. An empty job id is
polled forever, because `GetJobStatus("")` answers `unknown` rather than
`refused`. The refusal is not lost - it is replaced by a false statement about the
world, which is worse than an error the caller can render honestly.

zbus has the channel: an interface method returning `zbus::fdo::Result<T>` sends
an error reply instead of a value, and the success signature is unchanged, so no
caller's argument shape moves.

What this looks for: inside a `#[zbus::interface]` impl, a method that both
mentions a refusal AND returns an empty literal. Both conditions, because a method
that returns an empty list for an ordinary reason - nothing matched, nothing is
pending - is not doing anything wrong.

What it does NOT cover, and the list matters more than the count:

  * Socket protocols. The knowledge daemon answers an out-of-scope read with a
    uniform `OutOfScope` on purpose, so that out-of-scope and absent are
    indistinguishable to a caller probing for what exists. That is the opposite
    design decision, made deliberately, and it is not this.
  * Frontends. `apps/harness/src-tauri` turns any error from the agent into `"[]"`
    itself, which re-creates the empty list one layer above a daemon that now
    refuses honestly. Nothing here reads TypeScript or a Tauri command's fallback.
  * A refusal that returns an empty value through a helper rather than a literal
    `return` in the method body.

So a pass means no D-Bus method visibly answers a refusal with an empty literal,
not that every refusal in the system is honest.

**The second rule is the same principle in the other medium: a process exit.**
`arlen-run` refuses a launch by exiting non-zero, and the shell learns the reason
from a line beginning with `arlen-run` - nothing else. Nine argv-parse arms
returned a bare `Err(exit::BAD_ARGS)` and printed nothing, so the launcher's front
door was the one path that stops without a word: exit 64 with an empty stderr,
which the shell cannot tell from a program that exited 64 itself and therefore
shows as nothing at all. Same failure as the empty list, reached by a different
route - the refusal is not lost, it is replaced by silence.

Every arm goes through `bad_args` now. What this checks is that a TENTH one has to
as well, because both sides of that contract can pass their own tests while a new
refusal slips between them.
"""

import re
import sys
from pathlib import Path

# A tree to check may be passed in, which is what lets this gate's own test drive
# it against fixtures; the sibling gates take the same argument.
ROOT = (
    Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else Path(__file__).resolve().parents[2]
)

IFACE = re.compile(r"#\[(?:zbus::)?interface\b")
FN = re.compile(r"^\s{4}(?:pub )?(?:async )?fn (\w+)")
END = re.compile(r"^\s{4}\}\s*$")

# A method body that mentions one of these is on a refusal path.
REFUSAL = re.compile(r"refus|not admitted|not a user surface|AccessDenied|unauthoris|unauthoriz")
# ...and returning one of these instead of an error is the defect.
# Either an explicit `return`, or the value alone on its own line - Rust's tail
# expression, which is how anyone writing this fresh would write it. The first
# version required the `return` keyword and so could only see the form nobody
# reaches for: a method ending in a bare `Vec::new()` after a refusal warning
# passed it completely. Measured on 11 August - no method in the tree currently
# uses the implicit form, so this is about the next one, not a live break.
EMPTY = re.compile(
    r'(?:return\s+|^\s*)'
    r'(?:"\[\]"\.to_string\(\)|String::new\(\)|Vec::new\(\)|Default::default\(\)|"\[\]"|vec!\[\])'
    r"\s*;?\s*$",
    re.M,
)

# A method that answers a refusal with a value for a reason someone stands behind.
# Empty is the goal. An entry needs the reason, not just the name.
ACKNOWLEDGED: dict[str, str] = {}


# `Err(exit::BAD_ARGS)` written out rather than routed through the helper that
# prints. Matched on the literal because that is exactly the shape that was there
# nine times; a refusal reaching the code some other way is not something this can
# see, and saying so is better than implying it can.
BARE_EXIT = re.compile(r"Err\(exit::(\w+)\)")


def silent_launcher_refusals(root: Path) -> list[str]:
    """Argv refusals in `arlen-run` that return a code without printing a reason."""
    src = root / "daemons/arlen-run/src/main.rs"
    if not src.is_file():
        return []
    text = src.read_text(encoding="utf-8", errors="replace")
    # Stop at the test module. Its `assert_eq!(parse_args(..), Err(exit::BAD_ARGS))`
    # lines are the expectation, not a refusal site, and reading them as findings
    # would have this gate demand that the tests print - which is backwards, and is
    # what the first version of it did.
    cut = text.find("#[cfg(test)]")
    if cut != -1:
        text = text[:cut]
    out: list[str] = []
    for i, line in enumerate(text.splitlines(), 1):
        # A doc comment naming the shape it warns about is not the shape. The
        # helper's own doc quotes `Err(exit::BAD_ARGS)` to say what it replaced,
        # and reading that as a finding would make the fix trip its own check.
        if line.lstrip().startswith(("//", "*")):
            continue
        if BARE_EXIT.search(line) and "bad_args" not in line:
            out.append(f"daemons/arlen-run/src/main.rs:{i}")
    return out


def main() -> int:
    findings: list[str] = []
    methods = 0
    files = 0
    for path in sorted(ROOT.rglob("*.rs")):
        parts = set(path.parts)
        if {"target", "node_modules", "mkosi.builddir", ".git"} & parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if not IFACE.search(text):
            continue
        files += 1
        lines = text.splitlines()
        inside = False
        name = ""
        body: list[str] = []
        start = 0
        for i, line in enumerate(lines):
            m = FN.match(line)
            if m:
                inside, name, body, start = True, m.group(1), [], i + 1
                continue
            if inside and END.match(line):
                methods += 1
                joined = "\n".join(body)
                if REFUSAL.search(joined) and EMPTY.search(joined):
                    key = f"{path.relative_to(ROOT)}::{name}"
                    if key not in ACKNOWLEDGED:
                        findings.append(
                            f"{path.relative_to(ROOT)}:{start}: `{name}` answers a refused "
                            f"caller with an empty value. Return `zbus::fdo::Result` and "
                            f"an `AccessDenied` instead - the success signature does not change."
                        )
                inside = False
                continue
            if inside:
                body.append(line)

    exits = silent_launcher_refusals(ROOT)
    for site in exits:
        findings.append(
            f"{site}: refuses with a bare exit code and prints nothing, so the shell "
            f"has no line to show and the launch stops in silence. Route it through "
            f"`bad_args(reason)`, which prints and returns the same code."
        )

    print(
        f"{methods} D-Bus method(s) across {files} file(s) checked for a refusal that "
        f"answers with a value, and the launcher's argv refusals checked for one that "
        f"exits without a word. Reads Rust interface bodies only: a socket protocol's "
        f"uniform denial is a different and deliberate design, and a frontend's own "
        f"error fallback is not visible here."
    )
    if findings:
        print("\nrefusals that answer with a value instead of an error:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
