# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every executor that ACTS gates on `executor_live`.

`executor_live` is the flag that lets the AI act on the real system: move and
trash the user's files, write their settings, run commands, write the knowledge
graph. It defaults off, and every executor that acts reads it through an
injected `fn() -> bool` so a test can drive both sides.

The failure this exists for is not a wrong gate, it is a MISSING one. A new
sub-executor is a file that looks exactly like its neighbours, registers itself
with the router, and does its work; nothing fails if it never asks the flag. It
would simply act while the system believes the AI cannot. Nothing else catches
that: the router only dispatches, the tests of the new executor would pass, and
the flag being off is precisely the state in which nobody looks.

So: every `execute*` method in an executor file either contains the gate, or is
listed below as one that legitimately does not.
"""

import pathlib
import re
import sys

# A tree to check may be passed in, which is what lets this gate's own test drive
# it against fixtures; the sibling gates take the same argument.
ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)
EXECUTORS = ROOT / "daemons/ai-engine-daemon/src"

# `async fn execute(...)`, `async fn execute_move(...)` - the acting entry points.
FN = re.compile(r"^\s+(?:pub )?(?:async )?fn (execute[_a-z]*)\s*\(", re.M)
GATE = re.compile(r"if !\(self\.executor_live\)\(\)")
# What makes a file an executor: it implements the trait. The `Executor` trait is
# declared in `dispatch.rs`, so a file that only mentions the word is not one.
IMPLEMENTS_EXECUTOR = re.compile(r"^impl\s+Executor\s+for\s", re.M)

# file -> (methods, why acting without the flag is right). Every ungated
# `execute*` must be here. This list shrinking is not the goal; the list being
# ACCURATE is, because each entry is a claim that the method does not act.
ACKNOWLEDGED: dict[str, tuple[set[str], str]] = {
    "proxy_executor.rs": (
        {"execute"},
        "a router: it owns no side effect and dispatches to the sub-executor "
        "registered for the tool, each of which gates itself",
    ),
    "read_executor.rs": (
        {"execute"},
        "a read, bounded by the grant's read scope; the flag gates ACTING on the "
        "system, and refusing reads under it would disable the assistant "
        "entirely rather than make it safe",
    ),
    "file_executor.rs": (
        {"execute"},
        "the dispatcher for this executor's three operations (move, trash, "
        "create), each of which gates before it touches anything",
    ),
    "placeholder.rs": (
        {"execute"},
        "`UnavailableExecutor` runs nothing: every call returns Unavailable with "
        "the not-wired reason, so there is no act for the flag to withhold. It is "
        "here at all because this check used to find executors by filename and "
        "never saw it - the entry is the record that discovery is structural now, "
        "and it goes when the trusted privileged-tool runner replaces the stub",
    ),
}


def production_text(text: str) -> str:
    """The file with every `#[cfg(test)]` / `mod tests` scope blanked out.

    Production only. A `#[cfg(test)]` module holds test doubles that implement the
    same trait and never touch the system, and counting them would let an
    acknowledged name cover a real executor that happens to share it.

    Follows braces rather than cutting at the first marker. Cutting drops the REST
    OF THE FILE, so a sub-executor written below the test module would be invisible
    - and a new sub-executor that never asks the flag is the entire failure this
    check exists for. Nothing is hidden there today (measured on 11 August, no
    `execute*` after a cut outside a test scope); this is about the one that gets
    added later.
    """
    lines = text.splitlines()
    inside, depth, opened, pending = set(), 0, [], False
    for i, line in enumerate(lines):
        if opened:
            inside.add(i)
        if "#[cfg(test)]" in line or line.strip().startswith("mod tests"):
            pending = True
        for ch in line:
            if ch == "{":
                depth += 1
                if pending:
                    opened.append(depth)
                    pending = False
            elif ch == "}":
                if opened and depth == opened[-1]:
                    opened.pop()
                depth -= 1
    return "\n".join("" if i in inside else l for i, l in enumerate(lines))


def acting_methods(path: pathlib.Path) -> list[tuple[str, bool]]:
    """Every `execute*` in the file, with whether its body contains the gate.

    A method's body runs to the next method at the same indent or the end of the
    file, which is enough here: these files are one impl block of flat methods,
    and a nested helper would still be inside the body it belongs to.
    """
    text = production_text(path.read_text())
    hits = list(FN.finditer(text))
    if not hits:
        return []
    out: list[tuple[str, bool]] = []
    for i, m in enumerate(hits):
        end = hits[i + 1].start() if i + 1 < len(hits) else len(text)
        body = text[m.end() : end]
        out.append((m.group(1), bool(GATE.search(body))))
    return out


def main() -> int:
    # Discover by what a file IS, not by what it is called. This used to glob
    # `*executor*.rs`, and `placeholder.rs` holds a production `impl Executor for`
    # that the name never matched - inert today (it answers Unavailable to
    # everything), which is exactly why it sat there unnoticed proving the naming
    # convention is enforced by nothing. The next one need not be inert.
    files = sorted(
        p
        for p in EXECUTORS.glob("*.rs")
        if IMPLEMENTS_EXECUTOR.search(production_text(p.read_text()))
    )
    if not files:
        sys.exit(f"found no executor files under {EXECUTORS}; the check needs updating")

    problems: list[str] = []
    checked = ungated = 0
    for path in files:
        methods = acting_methods(path)
        if not methods:
            continue
        allowed, _ = ACKNOWLEDGED.get(path.name, (set(), ""))
        seen_ungated = set()
        for name, gated in methods:
            # A test double inside `#[cfg(test)]` is not an executor of the system.
            checked += 1
            if gated:
                continue
            ungated += 1
            seen_ungated.add(name)
            if name not in allowed:
                problems.append(
                    f"{path.name}: {name} does not gate on executor_live, so it would act "
                    "while the system believes the AI cannot. Add the gate, or list it with "
                    "the reason it does not act."
                )
        for stale in sorted(allowed - seen_ungated):
            problems.append(
                f"{path.name}: {stale} is listed as not acting but now gates (or is gone). "
                "Delete the entry so the list keeps meaning something."
            )

    for name in sorted(set(ACKNOWLEDGED) - {f.name for f in files}):
        problems.append(f"{name} is listed but is not an executor file any more; delete the entry")

    if problems:
        print("an executor can act with the AI switched off:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} executor entry point(s) across {len(files)} file(s); "
        f"{ungated} do not gate and each is acknowledged"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
