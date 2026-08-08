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
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

CATCH = re.compile(r"\}\s*catch\b[^{]*\{", re.S)
STORE_WRITE = re.compile(r"\.(set|update)\(")
COMMENT = re.compile(r"/\*.*?\*/", re.S)

SKIP = ("/harness/", "/store/", "node_modules")

# The ones that were already there, worst first. This is a queue, not an alibi:
# every line is a button that lies when its backend is down, and most of them have
# no backend at all (see check-invoke-exists.py). An entry leaves this list when
# the action is made honest, not when it is explained.
KNOWN: dict[str, str] = {
    "apps/settings/src/lib/stores/windows-apps.ts": (
        "installExe, where the file picker and a failed install are the same "
        "exception - the same problem as installThemeFile"
    ),
    "apps/settings/src/lib/stores/models.ts": (
        "setRole, startDownload, cancelDownload - arlen-ui's model picker is live "
        "work; named rather than skipped because the shape is identical"
    ),
    "apps/settings/src/lib/stores/appSettings.ts": (
        "clearAppCache, which reports a cache cleared that was not"
    ),
    "apps/settings/src/lib/stores/themes.ts": (
        "installThemeFile, where a cancelled file picker and a failed install "
        "arrive as the same exception - telling them apart needs an error shape "
        "the command does not return yet"
    ),
    "apps/desktop-shell/src/lib/stores/printDialog.ts": (
        "submitPrint and cancelPrint, whose commands are also missing"
    ),
    "apps/desktop-shell/src/lib/stores/windowsFile.ts": "run and install",
    "apps/desktop-shell/src/lib/stores/waypointerAsk.ts": "escalate",
    "apps/knowledge/src/lib/stores/search.ts": "saving a search",
    "apps/meetings/src/lib/stores/meeting.ts": "saveNotes and two others",
    "apps/text-editor/src/lib/stores/lens.ts": "openRelated, which navigates rather than mutates",
}


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


def main() -> int:
    findings: list[str] = []
    known_hits = 0
    checked = 0
    files = sorted((ROOT / "apps").rglob("*.ts")) + sorted((ROOT / "apps").rglob("*.svelte"))
    for path in files:
        s = str(path)
        if any(k in s for k in SKIP) or "/src/" not in s:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if "invoke(" not in text:
            continue
        rel = str(path.relative_to(ROOT))
        for start, body_start, end in catch_spans(text):
            checked += 1
            body = COMMENT.sub("", "\n".join(l.split("//")[0] for l in text[body_start:end].splitlines()))
            if body.strip():
                continue
            head = text[:start]
            try_at = head.rfind("try")
            if try_at < 0 or "invoke(" not in text[try_at:start]:
                continue
            if not STORE_WRITE.search(head[max(0, try_at - 500) : try_at]):
                continue
            if rel in KNOWN:
                known_hits += 1
                continue
            line = head.count("\n") + 1
            findings.append(
                f"{rel}:{line}: a store was updated optimistically, the call failed, "
                f"and the catch does nothing - so the surface states that something "
                f"happened which did not. Keep the optimism under "
                f"`import.meta.env.DEV`, revert in a real session, and say so where "
                f"the claim is made."
            )

    print(
        f"{checked} catch block(s) checked for a mutation that failed silently after "
        f"an optimistic update. {known_hits} in {len(KNOWN)} known file(s) carried as "
        f"a queue. Cannot see a revert's correctness, a component-level optimistic "
        f"write, or a catch that only logs."
    )
    if findings:
        print("\nactions that claim to have happened when they did not:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
