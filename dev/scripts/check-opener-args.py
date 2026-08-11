# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every `xdg-open` call ends its options before the path.

A file may legally be named `-report.pdf`, and `xdg-open` parses a leading-dash
argument as its own option. Measured on this machine, whose `xdg-open` is
`handlr`:

    $ xdg-open -zzz-nonexistent
    error: unexpected argument '-z' found
      tip: to pass '-z' as a value, use '-- -z'

So the file never opens, and the caller reports a failure that names an argument
the user did not type. `--` before the value fixes it, and the tool prescribes
exactly that in its own error text.

This is deliberately about ONE tool. The obvious generalisation is wrong: the
same guard applied to `nmcli` breaks it, because nmcli does not honour `--` as an
end-of-options marker and would take it as the connection name -

    $ nmcli connection show -- "-p"
    Error: -- - no such connection profile.

- while it also does not parse a leading-dash positional as an option in the
first place, so it needs no guard. Both halves were measured before this file
existed. If you are tempted to widen this check to another command, run the two
commands against that command first; the general principle is not evidence about
any particular tool.

What this does NOT cover:

  * `xdg-mime`, `gio`, `gtk-launch` and the rest. Unmeasured, so unlisted: a
    check that guesses is worse than one that admits its scope.
  * A path that reaches `xdg-open` through a helper rather than a literal
    `.arg(...)` on the same call.
  * Whether an acknowledged call's witness string really makes it safe. The
    witness ties the excuse to one call rather than to a whole file; reading
    whether `abs(` does what its entry claims is still a person's job.
  * Whether a dash-named file actually opens end to end. That needs a real
    session and an application launch, which is not something CI or an agent
    should be doing on somebody's desktop. The parse is what is verified.
"""

import re
import sys
from pathlib import Path

# The tree to scan. An argument so this can be pointed at a fixture and shown
# to fail: a check that only ever runs against a tree that already passes
# cannot demonstrate the defect it exists for (standing rule, 11 Aug).
ROOT = (
    Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else Path(__file__).resolve().parents[2]
)

CALL = re.compile(r'Command::new\("xdg-open"\)')
# The `--` may sit on its own `.arg("--")` line before the value.
GUARD = re.compile(r'\.arg\("--"\)')

# A call that passes something which cannot look like an option: the file, the
# string that must appear in THAT call's builder chain for the excuse to hold,
# and why. Empty is not the goal here - an argument that is absolute by
# construction is better than a guard, and saying so is the point.
#
# The witness string matters. This was a plain file allowlist until 9 August,
# when adding a second, unguarded `xdg-open` call to `apps/files` to re-prove the
# check found that the check said nothing: the file was excused as a whole, so
# any NEW call in it inherited the excuse. A file-keyed exception is a hole that
# grows.
ACKNOWLEDGED: dict[str, tuple[str, str]] = {
    "apps/files/src-tauri/src/lib.rs": (
        "abs(",
        "Passes `abs(&path)`, which trims leading slashes and re-adds one, so the "
        "argument always starts with `/` and can never be read as an option. "
        "Checked the function, not the name.",
    ),
    "apps/harness/src-tauri/src/file_ref.rs": (
        ".arg(",
        "arlen-ui's in-flight work. Named rather than skipped, because the call "
        "has the same shape as the ones that were fixed.",
    ),
}


def main() -> int:
    findings: list[str] = []
    acknowledged: list[str] = []
    calls = 0
    scanned = 0

    for path in sorted(ROOT.rglob("*.rs")):
        parts = set(path.parts)
        if {"target", "node_modules", ".git", "mkosi.builddir"} & parts:
            continue
        scanned += 1
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in CALL.finditer(text):
            calls += 1
            # The builder chain: from the call to the first `.spawn()`/`.output()`.
            tail = text[m.end() : m.end() + 600]
            end = min(
                (i for i in (tail.find(".spawn()"), tail.find(".output()")) if i != -1),
                default=len(tail),
            )
            chain = tail[:end]
            if GUARD.search(chain):
                continue
            rel = str(path.relative_to(ROOT))
            excuse = ACKNOWLEDGED.get(rel)
            # The excuse applies to a call that carries its witness, not to every
            # call the file will ever contain.
            if excuse and excuse[0] in chain:
                acknowledged.append(f"{rel}: {excuse[1]}")
                continue
            line = text[: m.start()].count("\n") + 1
            findings.append(
                f"{rel}:{line}: passes its argument to xdg-open without a `--` "
                f"first, so a file named `-something` is read as options and never "
                f"opens"
            )

    # Found the moment this check was first pointed somewhere other than the
    # tree it was written against: it answered "pass" to a directory with
    # nothing in it. A count of zero is only honest if there was something to
    # count - no Rust sources at all means the layout moved and this check went
    # quiet, which is the one failure a green result must never be able to hide.
    if scanned == 0:
        print("found no Rust sources to scan; the layout moved and this check went quiet")
        return 1

    print(
        f"{calls} xdg-open call(s) across {scanned} file(s) checked for an "
        f"end-of-options marker. "
        f"One tool only: the same guard breaks nmcli, which was measured before "
        f"this check was written."
    )
    if acknowledged:
        print("\nunguarded for a reason:\n")
        for a in sorted(set(acknowledged)):
            print(f"  - {a}")
    if findings:
        print("\nopener calls a dash-leading name would defeat:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
