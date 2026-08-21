# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a spawn of a tool the image does not ship says so in words.

`dev/scripts/runtime-deps.tsv` records which external programs the image
actually carries and which it does not. For an ABSENT one, failing to start it
is not a rare fault on the machine we ship: it is the ordinary state. So what a
person reads when they press the button is whatever the spawn's error path
produces, and on 21 August that was the errno.

  nmcli connect failed: No such file or directory (os error 2)
  wpctl not found: No such file or directory (os error 2)

The first reads as the connection failing and names a file nobody mentioned. The
second names a program the person has never heard of and does not say what
stopped working. Both send somebody to retry something the machine cannot do at
all. Twenty-four of them, in audio, network, power and the clipboard.

WHAT IS IN SCOPE, and why it is drawn this narrowly:

A spawn of an ABSENT tool whose error is turned into a String - `map_err` with
`e.to_string()` or a `format!` carrying `{e}` - because that String is what a
command hands back to a surface. A site that swallows into a default instead
(`_ => return String::new()`) produces no wrong sentence, and turning those into
errors is a different change with a different argument behind it, so they are
not findings here.

A site is satisfied by anything that distinguishes absence: a branch on
`ErrorKind::NotFound`, or a call to a helper that does (the shell's
`missing_tool::tool_error`). The check does not read the sentence, only whether
the code can tell "not installed" from "failed" - a message nobody can act on is
a matter for whoever writes it, and this is the part a machine can hold.

Run: dev/scripts/check-missing-tool-message.py [root]
"""

import re
import subprocess
import sys
from pathlib import Path

#: Turning the error into a String the caller shows a person.
INTO_MESSAGE = re.compile(r"map_err\(\|e\| (?:e\.to_string\(\)|format!\(\"[^\"]*\{e\})")

#: Anything that can tell an absent tool from a failing one.
HANDLES_ABSENCE = ("NotFound", "not installed", "tool_error")

#: A hard cap on how far past the spawn to read, for the pathological case of a
#: statement that never ends. The real bound is the statement itself: the first
#: version read a fixed 700 characters and reported `bluetooth.rs`, where the
#: spawn is a deliberate `let _ = ...` and the `map_err` it found belonged to the
#: NEXT statement, about a D-Bus connection. A window that spans past the end of
#: the thing being judged reports on its neighbour.
WINDOW = 4000


def absent_tools(root: Path) -> set[str]:
    """The tools `runtime-deps.tsv` says the image does not carry."""
    out = set()
    tsv = root / "dev/scripts/runtime-deps.tsv"
    for line in tsv.read_text().splitlines():
        if line.startswith("#") or not line.strip():
            continue
        fields = line.split("\t")
        if len(fields) > 3 and fields[3].strip() == "absent":
            out.add(fields[0].strip())
    return out


def statement_after(text: str, start: int) -> str:
    """The rest of the statement the spawn is part of, up to its `;`."""
    end = text.find(";", start)
    if end == -1:
        end = start + WINDOW
    return text[start : min(end, start + WINDOW)]


def tracked_rust(root: Path) -> list[Path]:
    listed = subprocess.run(
        ["git", "ls-files", "*.rs"], cwd=root, capture_output=True, text=True
    ).stdout.split()
    return [root / p for p in listed if "/target/" not in p]


def main(argv: list[str]) -> int:
    root = Path(argv[1] if len(argv) > 1 else ".").resolve()
    absent = absent_tools(root)
    if not absent:
        print("no absent tools declared; runtime-deps.tsv moved and this check did not")
        return 1

    spawn = re.compile(r'Command::new\("(' + "|".join(re.escape(t) for t in absent) + r')"\)')
    findings: list[str] = []
    checked = 0
    for path in tracked_rust(root):
        try:
            text = path.read_text()
        except (OSError, UnicodeDecodeError):
            continue
        for m in spawn.finditer(text):
            checked += 1
            tail = statement_after(text, m.end())
            if any(h in tail for h in HANDLES_ABSENCE):
                continue
            if not INTO_MESSAGE.search(tail):
                continue
            line = text[: m.start()].count("\n") + 1
            findings.append(
                f"  {path.relative_to(root)}:{line} spawns `{m.group(1)}`, which the image "
                f"does not carry, and hands the error straight on as the message."
            )

    if findings:
        print("a missing program reported as an errno:\n")
        print("\n".join(sorted(findings)))
        print(
            "\nThe image ships none of these tools, so this is what a person reads when "
            "they press the button - not a rare fault. Say what stopped working, then "
            "name the program; keep the errno for a failure that is not an absence."
        )
        return 1

    print(
        f"{checked} spawn(s) of a tool the image does not carry: each either tells an "
        f"absent program from a failing one, or does not put the error in front of anybody."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
