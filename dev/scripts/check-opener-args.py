# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a path handed to a desktop tool cannot be read as an option.

A file may legally be named `-report.pdf`, and `xdg-open` parses a leading-dash
argument as its own option. Measured on this machine, whose `xdg-open` is
`handlr`:

    $ xdg-open -zzz-nonexistent
    error: unexpected argument '-z' found
      tip: to pass '-z' as a value, use '-- -z'

So the file never opens, and the caller reports a failure that names an argument
the user did not type. `--` before the value fixes it, and the tool prescribes
exactly that in its own error text.

Three tools now, each measured on its own, because the remedy is NOT the same for
all of them - `xdg-open` and `xdg-mime` REJECT `--` and need an absolute argument,
while `gtk-launch` requires `--` and has no path to make absolute. One line copied
from either to the other breaks it. The obvious generalisation is wrong: the
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

  * `gio` and the rest. Unmeasured, so unlisted: a check that guesses is worse
    than one that admits its scope. `xdg-mime` and `gtk-launch` were both on this
    line until 12 Aug, when running them took a minute each - the rule is to
    measure before widening, not to leave a tool out forever. `gio` stays here
    because nothing in the tree spawns it, so measuring it would be scope for its
    own sake.
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

# Both tools in the family that have been MEASURED to take a leading-dash
# argument as an option. `xdg-mime` joined on 12 Aug:
#
#     $ xdg-mime query filetype -x.txt
#     xdg-mime: unexpected option '-x.txt'
#
# It was in this file's not-covered list under "unmeasured, so unlisted - a check
# that guesses is worse than one that admits its scope", which was the right rule
# and the wrong conclusion to leave sitting: the tree asks `xdg-mime` from three
# places, and one of them was passing a path straight off the launch socket. The
# rule stands for `gio` and `gtk-launch`, which remain unmeasured and unlisted.
CALL = re.compile(r'Command::new\("xdg-(?:open|mime)"\)')

# How far back to look for the step that makes the argument absolute. It reads
# before the call far more often than inside the builder chain -
# `let path = canonicalize(path)?;` on the line above, `let abs = abs(&path);` two
# lines above - so a chain-only search reported every one of them. Bounded rather
# than whole-function on purpose: a wide window starts finding somebody else's
# `canonicalize` and calling this call safe, which is the failure mode that costs
# more than a false positive.
LOOKBEHIND = 320

# `.arg("--")` is NOT a guard here, and used to be the one this gate asked for.
#
# Measured against `/usr/bin/xdg-open` (xdg-utils 1.2.1) on 12 Aug, because the
# remedy had never been run:
#
#     $ xdg-open -- /nonexistent
#     xdg-open: unexpected option '--'
#
# Its operative argument loop is `case "$parm" in -*) exit_failure_syntax`, and
# `--` matches `-*`. There IS a `--)` arm further up, but it sits in
# `check_common_commands`, a pre-scan for `--help`, and it only breaks when
# `XDG_UTILS_ENABLE_DOUBLE_HYPEN` is set, which nothing sets. So the end-of-options
# convention this gate was built on is not implemented by the tool it is about, and
# following the advice would have broken opening entirely rather than hardened it.
#
# Nobody was hurt only by luck: both real call sites reach `xdg-open` with an
# argument that is absolute by construction and were carried as exemptions, so the
# recommended fix was never actually applied to anything.
#
# The rule is therefore the one this file's exemption note already called better:
# **make the argument absolute**, so it cannot be read as an option in the first
# place. A marker is now a finding of its own.
BREAKS = re.compile(r'\.arg\("--"\)')

# The other half of the family, and the reason this file keeps insisting on
# measurement: `gtk-launch` takes the OPPOSITE fix.
#
#     $ gtk-launch -x.desktop
#     Error parsing commandline options: Unknown option -x.desktop
#     $ gtk-launch -- -x.desktop
#     gtk-launch: no such application -x.desktop
#
# With the marker the argument reaches the lookup, so GLib's parser honours what
# xdg-utils rejects. Three tools, two opposite remedies, one of which would break
# the other's calls - which is what "the general principle is not evidence about
# any particular tool" means in practice.
#
# It is also the case where the absolute-path answer does not exist: the argument
# is a desktop-entry ID, not a path, so there is nothing to make absolute and the
# marker is the only fix available.
MARKER_CALL = re.compile(r'Command::new\("gtk-launch"\)')

# What genuinely makes a call safe: the value cannot begin with a dash because it
# was made absolute first. Matched in the builder chain, and the exemption list
# below carries the cases where the absolute step happens in the caller.
GUARD = re.compile(r"canonicalize|absolute|\babs\(")

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
# Empty, and it emptied the right way: the entry here excused the harness's
# `spawn_xdg_open`, and that call is gone - the harness asks the launch socket
# now, so there is no argument for a guard to be absent from. The staleness check
# below is what said so, on the commit that removed it, rather than leaving a
# fixed call described as open for however long nobody looked.
ACKNOWLEDGED: dict[str, tuple[str, str]] = {}

# `apps/files/src-tauri/src/lib.rs` used to need an entry here for `abs(&path)`.
# It does not any more: `abs(` is now part of the guard itself, since making the
# argument absolute IS the fix rather than a reason to be excused from one.


def builder_chain(text: str, end_of_call: int) -> str:
    """The builder chain after a `Command::new(...)`, with comments removed.

    Comments come out first, and that is not tidiness. The window used to be 600
    raw characters, which a call site can exceed with prose alone: writing a
    twelve-line note above `.arg("--")` in `app_search.rs` pushed the argument out
    of the window and the gate reported the call it had just been taught to
    accept. A scanner that a comment can defeat measures how much someone wrote,
    not what the code does.
    """
    window = text[end_of_call : end_of_call + 2000]
    code = "\n".join(line.split("//")[0] for line in window.splitlines())
    end = min(
        (i for i in (code.find(".spawn()"), code.find(".output()")) if i != -1),
        default=len(code),
    )
    return code[:end]


def main() -> int:
    findings: list[str] = []
    acknowledged: list[str] = []
    # Calls carrying `--`. Reported loudly, deliberately NOT failing - see the note
    # where they are printed.
    breaks: list[str] = []
    # Entries that actually excused a call this run. One that excused nothing has
    # stopped describing the tree - see after the loop.
    used: set[str] = set()
    calls = 0
    scanned = 0

    for path in sorted(ROOT.rglob("*.rs")):
        parts = set(path.parts)
        if {"target", "node_modules", ".git", "mkosi.builddir"} & parts:
            continue
        scanned += 1
        text = path.read_text(encoding="utf-8", errors="replace")

        # The marker family, checked first and by the opposite rule: here `--` is
        # required rather than forbidden.
        for m in MARKER_CALL.finditer(text):
            calls += 1
            if not BREAKS.search(builder_chain(text, m.end())):
                line = text[: m.start()].count("\n") + 1
                rel = str(path.relative_to(ROOT))
                findings.append(
                    f"{rel}:{line}: passes an argument to gtk-launch without a `--` "
                    f"first, so an entry named `-something.desktop` is read as an "
                    f"unknown option and never launches. This tool DOES honour the "
                    f"marker, unlike the openers above - measured, not assumed"
                )

        for m in CALL.finditer(text):
            calls += 1
            chain = builder_chain(text, m.end())
            before = text[max(0, m.start() - LOOKBEHIND) : m.start()]
            rel = str(path.relative_to(ROOT))
            # Checked BEFORE the guard and before any excuse: a marker is a broken
            # call whatever else the chain does, so it must not be able to hide
            # behind either.
            if BREAKS.search(chain):
                line = text[: m.start()].count("\n") + 1
                breaks.append(f"{rel}:{line}")
                continue
            if GUARD.search(chain) or GUARD.search(before):
                continue
            excuse = ACKNOWLEDGED.get(rel)
            # The excuse applies to a call that carries its witness, not to every
            # call the file will ever contain.
            if excuse and excuse[0] in chain:
                acknowledged.append(f"{rel}: {excuse[1]}")
                used.add(rel)
                continue
            line = text[: m.start()].count("\n") + 1
            findings.append(
                f"{rel}:{line}: passes an argument to xdg-open that may begin with "
                f"a dash, so a file named `-something` is read as options and never "
                f"opens. Make it absolute before the call - `--` is NOT the fix "
                f"here, xdg-utils rejects it"
            )

    # Found the moment this check was first pointed somewhere other than the
    # tree it was written against: it answered "pass" to a directory with
    # nothing in it. A count of zero is only honest if there was something to
    # count - no Rust sources at all means the layout moved and this check went
    # quiet, which is the one failure a green result must never be able to hide.
    if scanned == 0:
        print("found no Rust sources to scan; the layout moved and this check went quiet")
        return 1

    # An excuse that excused nothing. Each entry says a specific call is unguarded
    # and why that is acceptable; once the call gains its `--`, or the file goes,
    # the sentence is false and reads as a known hole that is still open. Fourth
    # list to get this on 12 Aug, after `check-invoke-scope.py` turned out to be
    # carrying two acknowledgements of calls that had both been fixed.
    for rel in sorted(set(ACKNOWLEDGED) - used):
        findings.append(
            f"{rel} is acknowledged as passing an unguarded argument, but no such "
            f"call is there now - it gained its `--`, or the file moved. Drop the "
            f"entry rather than leave a fixed call described as open."
        )

    print(
        f"{calls} xdg-open/xdg-mime call(s) across {scanned} file(s) checked for an "
        f"argument that cannot be read as an option. "
        f"Two tools, both measured: the reasoning does not transfer on its own, "
        f"which nmcli demonstrated before this check was written."
    )
    if breaks:
        print(
            f"\n{len(breaks)} call(s) pass `--`, which xdg-utils REJECTS "
            f"(measured 12 Aug: `xdg-open -- /x` gives \"unexpected option '--'\"):\n"
        )
        for b in breaks:
            print(f"  - {b}")
        print(
            "\n  Reported rather than failed, because the fix is a packaging\n"
            "  decision this check cannot make. The `--` came from a dev machine's\n"
            "  personal shim - `~/.local/bin/xdg-open` execs `handlr open`, whose\n"
            "  clap parser DOES honour `--`, and the error text quoted in those call\n"
            "  comments is clap's. Against xdg-utils it is a call that opens nothing.\n"
            "  The image installs NEITHER, so nothing opens there today either way.\n"
            "  Settle which opener the image ships, then this becomes a failure or\n"
            "  the marker becomes correct - one answer, seven call sites."
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
