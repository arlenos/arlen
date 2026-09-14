#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""A daemon under test must not reach the developer's home.

The integration harness spawns real daemons, and a daemon resolves its paths
from the environment. Every variable the harness forgets is one the daemon reads
from the real session instead - so the test writes into the machine it is meant
to leave alone, and reads state the scenario never seeded.

FOUR OF THESE WERE FOUND ONE AT A TIME, each by someone tripping over it:

  * `XDG_CONFIG_HOME` - the knowledge daemon read the real `graph.toml`, found
    none, and scanned the developer's actual repositories on every test;
  * `XDG_DATA_HOME` - the audit daemon's HMAC key and ledger went to the real
    `~/.local/share`;
  * `ARLEN_AI_CONFIG` - the agent reads `$HOME/.config/arlen/ai.toml` directly
    rather than through XDG, so the XDG fix did not cover it;
  * `XDG_STATE_HOME` - the undo signer MINTED A SIGNING KEY in the real
    `~/.local/state`, which is exactly what its own custody rule exists to
    notice.

Four is enough to stop finding the fifth by accident. `HOME` is now set too,
which is the floor under all of them: an XDG variable nobody thought of falls
back to `$HOME/.local/...` and therefore lands inside the temp root anyway.

WHAT IS COMPARED. Every `env::var("X")` / `var_os("X")` under `daemons/*/src`
where X is `HOME` or `XDG_*`, against the keys `base_env` inserts. A variable that
is deliberately left to the real session is named in `READ_ONLY_SYSTEM` with its
reason - the test is whether somebody DECIDED, not whether the list is short.

Only those two families, deliberately. The `ARLEN_*` variables are this project's
own and mostly name a socket the harness already points somewhere; the leaks that
cost real time were all a path resolved from the home directory.
"""

import re
import sys
from pathlib import Path

OWN_TREE = len(sys.argv) <= 1
REPO = Path(__file__).resolve().parents[2] if OWN_TREE else Path(sys.argv[1]).resolve()

HARNESS = "dev/integration/src/lib.rs"
DAEMONS = "daemons"

#: `std::env::var_os("XDG_STATE_HOME")` and the `var` form.
READ = re.compile(r'\benv::var(?:_os)?\(\s*"(?P<name>HOME|XDG_[A-Z_]+)"')
#: `("XDG_STATE_HOME".to_string(), p("state")),` inside `base_env`.
SET = re.compile(r'\(\s*"(?P<name>HOME|XDG_[A-Z_]+)"\.to_string\(\)')

#: Variables a daemon may read from the real session, with the reason. These name
#: where the SYSTEM keeps things, not where the user's own state lives, so a test
#: that redirected them would be testing a machine with no system files on it.
READ_ONLY_SYSTEM = {
    "XDG_DATA_DIRS": "the system data roots (`/usr/share`), read to find installed "
    "sound themes; nothing is ever written there and an empty list would test a "
    "machine with no sounds installed rather than a hermetic one",
}


def read_by_daemons(repo: Path) -> dict[str, set[str]]:
    """Variable -> the daemon files that read it."""
    out: dict[str, set[str]] = {}
    root = repo / DAEMONS
    if not root.is_dir():
        return out
    for f in root.rglob("*.rs"):
        if "/target/" in str(f):
            continue
        text = f.read_text(encoding="utf-8", errors="replace")
        for m in READ.finditer(text):
            out.setdefault(m.group("name"), set()).add(str(f.relative_to(repo)))
    return out


def set_by_harness(repo: Path) -> set[str]:
    """The variables `base_env` puts in a spawned daemon's environment."""
    text = (repo / HARNESS).read_text(encoding="utf-8", errors="replace")
    start = text.find("fn base_env(")
    if start < 0:
        return set()
    # Bound to the function: the next line starting a new `pub fn` at this indent.
    rest = text[start:]
    end = rest.find("\n    pub fn ", 1)
    body = rest if end < 0 else rest[:end]
    return {m.group("name") for m in SET.finditer(body)}


def main() -> int:
    read = read_by_daemons(REPO)
    if not read:
        print(f"NOTHING WAS READ: no daemon under {DAEMONS} reads a home variable", file=sys.stderr)
        return 2
    have = set_by_harness(REPO)
    if not have:
        print(f"NOTHING WAS READ: `base_env` in {HARNESS} did not parse", file=sys.stderr)
        return 2

    missing = {v: f for v, f in read.items() if v not in have and v not in READ_ONLY_SYSTEM}
    if missing:
        print("Variables a daemon reads that the harness does not set:\n")
        for name, files in sorted(missing.items()):
            shown = sorted(files)[:3]
            print(f"  - {name}")
            for f in shown:
                print(f"      {f}")
            if len(files) > len(shown):
                print(f"      ... and {len(files) - len(shown)} more")
        print(
            "\nA daemon under test resolves this from the real session, so the run"
            "\nreads or writes the developer's own machine. Set it in `base_env`, or"
            "\nname it in READ_ONLY_SYSTEM with the reason it may stay real."
        )
        return 1

    print(
        f"the harness sets every home variable the daemons read "
        f"({len(read) - len(READ_ONLY_SYSTEM)} set, {len(READ_ONLY_SYSTEM)} left to the system by name)"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
