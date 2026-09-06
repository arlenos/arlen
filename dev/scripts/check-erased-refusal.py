# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a rollback does not erase the failure it is rolling back from.

The shape, found three times in Settings on 6 September:

    } catch (e) {
      inner.update((s) => ({ ...s, error: String(e) }));
      await load();              // roll the value back off disk
    }

    async function load() {
      inner.update((s) => ({ ...s, error: null }));   // ...and the record with it
      ...
    }

It reads as careful code, and half of it is: the surface was updated
optimistically, the write was refused, so the value is put back. What the second
line also does is destroy the only evidence that anything went wrong, one
statement after it was written, every time. The person sees the switch they just
moved slide back on its own with nothing said anywhere.

What it cost before it was named:

  * `createConfigStore.setValue`, which is most of fifteen Settings pages - every
    control that writes through the generic config store
  * `createConfigStore.reset`, the "put this back" button beside each row
  * the screen-filter store, so the invert-colours switch on the accessibility
    page snapped back silently - the page somebody is on precisely because the
    screen is hard to read

WHY `check-optimistic-write` CANNOT SEE IT. That gate asks whether a failed
mutation left the surface claiming success. Here it did not: the revert is
present and correct, which is all that check asks for, and it passes all three.
The defect is in what the revert does on its way past.

What it looks for: a `catch` that sets a field to an error value AND awaits a
same-file function whose body sets that same field back to null or false. Both
halves, because each alone is right - a catch that records is what you want, and
a loader that clears stale state before reading is what you want.

The fix that satisfies it is the one all three got: set the flag AFTER the
rollback, and give the write its own field so the loader has no reason to touch
it.

What it does NOT cover:

  * a loader in another module. Same defect, and the cross-file version needs an
    import graph this does not build
  * a field cleared by something other than an object literal - a `reset()`
    helper, a spread of a fresh initial state
  * whether the field is RENDERED. A flag nobody shows is a different check
    (`check-announced-refusal`)
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SKIP = ("/harness/", "/store/", "node_modules", "/.svelte-kit/")

CATCH = re.compile(r"catch\s*(?:\([^)]*\))?\s*\{")
#: A field being given a failure value: a stringified error, `true`, a message.
SETS = re.compile(r"\b(\w+)\s*:\s*(?:String\(|true\b|`|\"|')")
#: The same field being emptied.
CLEARS = re.compile(r"\b(\w+)\s*:\s*(?:null|false)\b")
AWAITS = re.compile(r"await\s+(\w+)\s*\(")
DECL = re.compile(r"(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*[^{;]*\{")

KNOWN: dict[str, tuple[int, str]] = {}


def block_at(text: str, brace: int) -> str:
    """The braced block starting at `brace`, or the rest of the text."""
    depth = 0
    for i in range(brace, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[brace : i + 1]
    return text[brace:]


def function_bodies(text: str) -> dict[str, str]:
    """Every named function's body, innermost definitions included."""
    out: dict[str, str] = {}
    for m in DECL.finditer(text):
        brace = text.find("{", m.end() - 1)
        if brace < 0:
            continue
        out[m.group(1)] = block_at(text, brace)
    return out


def findings_in(text: str) -> list[tuple[int, str, str, list[str]]]:
    """(line, loader, fields) for every catch that awaits its own erasure."""
    bodies = function_bodies(text)
    seen: set[int] = set()
    out: list[tuple[int, str, str, list[str]]] = []
    for c in CATCH.finditer(text):
        block = block_at(text, c.end() - 1)
        fields = {m.group(1) for m in SETS.finditer(block)}
        if not fields:
            continue
        for a in AWAITS.finditer(block):
            loader = bodies.get(a.group(1))
            if loader is None:
                continue
            both = sorted(fields & {m.group(1) for m in CLEARS.finditer(loader)})
            if not both:
                continue
            line = text[: c.start()].count("\n") + 1
            if line in seen:
                continue
            seen.add(line)
            out.append((line, a.group(1), a.group(1), both))
    return out


def sources() -> list[Path]:
    out: list[Path] = []
    for base in (ROOT / "apps",):
        for p in sorted(base.rglob("*.ts")) + sorted(base.rglob("*.svelte")):
            rel = "/" + str(p.relative_to(ROOT))
            if any(s in rel for s in SKIP) or p.name.endswith(".test.ts"):
                continue
            out.append(p)
    return out


def main() -> int:
    files = sources()
    bad: list[str] = []
    carried = 0
    for p in files:
        rel = str(p.relative_to(ROOT))
        hits = findings_in(p.read_text(encoding="utf-8", errors="replace"))
        allowed = KNOWN.get(rel, (0, ""))[0]
        if len(hits) > allowed:
            for line, loader, _, fields in hits:
                bad.append(
                    f"  - {rel}:{line}: records {', '.join(fields)} and then awaits"
                    f" `{loader}()`, which clears it again"
                )
        else:
            carried += len(hits)
    print(
        f"{len(files)} frontend source(s) checked for a rollback that erases the failure it"
        f" recovers from. {carried} carried in {len(KNOWN)} known file(s)."
        " A loader in another module is the same defect and is not visible here."
    )
    if bad:
        print("\nrefusals erased by their own rollback:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
