# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a host fixture answers a command with the WHOLE shape it promises.

The shape, found twice in one day on 6 September:

    if (cmd === "mail_open") {
      return Promise.resolve({ from: "...", subject: "...", text: "..." });
    }

`Message` has sixteen fields. Answering the seven the picture needs does not
produce a smaller picture, it produces a WRONG one: the reading surface iterates
`to`, `cc` and `attachments`, so `MessageView` threw while rendering, Svelte
bailed out of that subtree, and the pane kept the "pick a message" it had been
showing - beside a header that had already rendered its buttons from the same,
correct state. Two halves of one surface disagreeing, which is exactly what a
state bug looks like from outside, and it was written up as one before
instrumentation showed the fixture was the thing at fault.

The second instance was already committed and is the reason this is a gate rather
than a note. `screenshot-refuses-save.js` answered `list_outputs` with
`{name, width, height}` and no `index` - and the surface keys its source dropdown
on exactly that (`screen:${o.index}`, then `outputs.find((o) => o.index === n)`).
The option's LABEL read "eDP-1", so the picture looked right, and the value behind
it was `screen:undefined`.

WHY THIS IS WORTH A CHECK OF ITS OWN. A fixture is evidence. Every other gate here
protects the product from the tree; this one protects the MEASUREMENT from the
person taking it, and a wrong measurement is worse than none because it is acted
on. Both of the day's instances cost more time than the fix, and in one case
produced a report that named the wrong component.

What it looks for: a `cmd === "..."` branch whose `Promise.resolve` carries an
inline object literal (or an array of them), for a command whose Rust return type
this tree can resolve. Field names come from the same machinery
`check-invoke-shape` uses, so the two agree about what a command returns.

What it does NOT cover, and each of these is a SKIP rather than a guess:

  * an answer built by a helper or held in a named const - `files-refuses-op.js`
    builds its rows with an `entry(name, kind, size)` function, which no reading
    of the call site can expand. Counted as unread in the summary rather than
    reported, because a check that guesses about its own blind spot is worse than
    one that names it
  * a command whose return type this tree cannot resolve (a type alias, a generic,
    a struct in a crate the resolver does not walk)
  * whether the VALUES are plausible. A `width: 2` is a fine answer for a probe
    and a terrible one for a layout picture; only a person looking at the shot
    can say
  * optional fields. Rust `Option<T>` still names a field, and serde will happily
    fill a missing one with null - so this can ask for a key the surface never
    reads. That is the intended trade: a fixture that names every field it was
    given is one nobody has to reason about
"""

from __future__ import annotations

import importlib.util
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HOSTS = ROOT / "dev" / "screenshot" / "hosts"

BRANCH = re.compile(r'cmd\s*===\s*"([a-z0-9_]+)"')

# Fixtures that answer a command with a deliberately partial shape, with the
# reason. A queue, not an alibi: an entry leaves when the fixture answers whole.
KNOWN: dict[tuple[str, str], str] = {}


def shape_module():
    """The sibling check's Rust-side knowledge, so the two cannot disagree."""
    spec = importlib.util.spec_from_file_location(
        "invoke_shape", Path(__file__).with_name("check-invoke-shape.py")
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def js_keys(body: str) -> set[str]:
    """Top-level keys of a JS object literal, strings and nesting respected."""
    if not body.strip().startswith("{"):
        body = "{" + body + "}"
    out: set[str] = set()
    depth = 0
    i, n = 0, len(body)
    while i < n:
        c = body[i]
        if c in "{[(":
            depth += 1
        elif c in "}])":
            depth -= 1
        elif c in "\"'`":
            quote = c
            i += 1
            while i < n and body[i] != quote:
                i += 2 if body[i] == "\\" else 1
        elif depth == 1 and c == ":":
            j = i - 1
            while j >= 0 and body[j] in " \n\t":
                j -= 1
            k = j
            while k >= 0 and (body[k].isalnum() or body[k] in "_$\"'"):
                k -= 1
            name = body[k + 1 : j + 1].strip("\"'")
            if name:
                out.add(name)
        i += 1
    return out


def answered_keys(text: str, after: int, mod) -> set[str] | None:
    """The keys of the object this branch resolves, or None if it is not literal."""
    resolve = text.find("Promise.resolve", after)
    if resolve < 0 or resolve - after > 500:
        return None
    # The brace must BE the resolved value, reached over nothing but whitespace
    # and at most one `[`. Searching for the next `{` instead found the body of
    # the following `if` when a branch resolved a named const, and reported
    # `files-refuses-op.js` as answering one field of nine - a fixture that is
    # in fact fine and simply builds its rows with a helper.
    i = resolve + len("Promise.resolve")
    n = len(text)
    while i < n and text[i] in " \n\t":
        i += 1
    if i >= n or text[i] != "(":
        return None
    i += 1
    seen_bracket = False
    while i < n and (text[i] in " \n\t" or (text[i] == "[" and not seen_bracket)):
        seen_bracket = seen_bracket or text[i] == "["
        i += 1
    if i >= n or text[i] != "{":
        return None
    return js_keys(mod.balanced_body(text, i))


def main() -> int:
    mod = shape_module()
    returns = mod.rust_return_types(ROOT)
    fields = mod.rust_struct_fields(ROOT)[0]

    by_command: dict[str, str] = {}
    for cmds in returns.values():
        for command, struct in cmds.items():
            by_command.setdefault(command, struct)

    checked = 0
    unread = 0
    carried = 0
    bad: list[str] = []
    hosts = sorted(HOSTS.glob("*.js"))
    for path in hosts:
        text = path.read_text(encoding="utf-8", errors="replace")
        for branch in BRANCH.finditer(text):
            command = branch.group(1)
            struct = by_command.get(command)
            want = fields.get(struct) if struct else None
            if not want:
                continue
            got = answered_keys(text, branch.end(), mod)
            if got is None:
                unread += 1
                continue
            checked += 1
            missing = sorted(want - got)
            if not missing:
                continue
            if (path.name, command) in KNOWN:
                carried += 1
                continue
            line = text[: branch.start()].count("\n") + 1
            bad.append(
                f"  - {path.name}:{line}: `{command}` answers {len(got)} of"
                f" {len(want)} fields of `{struct}`, missing: {', '.join(missing)}"
            )

    print(
        f"{len(hosts)} host fixture(s); {checked} literal answer(s) compared against the"
        f" command's own return struct, {unread} built by a helper or a named const and"
        f" not read, {carried} carried."
        " Field names come from the same resolver `check-invoke-shape` uses."
        " Values are not judged: only a person looking at the shot can say whether a"
        " plausible-looking number is the right one."
    )
    if bad:
        print("\nfixtures answering less than the shape they promise:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
