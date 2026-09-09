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
  * optional fields, on purpose. `Option<T>` fields are NOT required: a real host
    with `skip_serializing_if` omits them, so demanding them makes a correct
    fixture fail. An early cut did exactly that to the jobs fixture
"""

from __future__ import annotations

import importlib.util
import re
import sys
from pathlib import Path

# An optional root, so the control can point this at a fixture tree instead of
# editing the repository. Its first cut did edit it - putting a real defect back,
# running, restoring - which is fine alone and wrong under the pre-commit hook,
# where the gates run CONCURRENTLY: another check reading the fixtures and the Rust structs
# mid-control sees a tree nobody wrote. It failed that way within the hour, and a
# gate that fails at random is worse than no gate.
ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
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


DECL = re.compile(r"(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*")

STRUCT = re.compile(r"\bstruct\s+([A-Za-z_]\w*)\s*\{")
FIELD = re.compile(r"(?:pub\s+)?([a-z_]\w*)\s*:\s*([^,\n]+)")
RENAME = re.compile(r'#\[serde\([^)]*\brename\s*=\s*"([^"]+)"')
RENAME_ALL = re.compile(r'#\[serde\([^)]*rename_all\s*=\s*"camelCase"')


def camel(name: str) -> str:
    head, *rest = name.split("_")
    return head + "".join(w[:1].upper() + w[1:] for w in rest)


def required_fields(root: Path, mod) -> dict[str, list[set[str]]]:
    """SCOPED, because struct names collide across a tree this size.

    An earlier cut walked every `.rs` under the repository root and took the first
    `struct Process` it found - which was not the system monitor's. The counts in
    the finding gave it away: eight of nine fields present and four named missing,
    which cannot both be true of one struct. Callers pass the app's own
    `src-tauri` now.
    """
    return _required_fields(root, mod)


def _required_fields(root: Path, mod) -> dict[str, list[set[str]]]:
    """Per struct, the fields a real host always sends.

    OPTION IS THE WHOLE POINT OF THIS FUNCTION. An earlier cut asked for every
    field the struct declares and immediately demanded `error` and `egress_host`
    from a jobs fixture - both `Option<String>` with `skip_serializing_if`, which
    a real host omits, so the fixture was right and the check was wrong. Asking
    only for the non-optional ones makes the question exact: a missing one of
    those would fail deserialization against a real host, which is precisely how
    the mail fixture broke the reading surface.
    """
    out: dict[str, list[set[str]]] = {}
    for path in root.rglob("*.rs"):
        if "/target/" in str(path) or "/node_modules/" in str(path):
            continue
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for m in STRUCT.finditer(text):
            body = mod.balanced_body(text, text.index("{", m.end() - 1))
            # THE WIRE NAME, not the Rust one. A fixture writes what serde puts
            # on the wire, and this tree uses all three conventions: the mail DTO
            # ships its fields verbatim (`has_html`), the system monitor renames
            # three of them by hand (`#[serde(rename = "memMB")]`), and others
            # carry `rename_all = "camelCase"`. An earlier cut snake-cased the
            # fixture's keys instead and reported `mem_m_b` missing from an answer
            # that carried `memMB`.
            head = text[max(0, m.start() - 400) : m.start()]
            all_camel = bool(RENAME_ALL.search(head))
            names: set[str] = set()
            pending: str | None = None
            for raw in body.split("\n"):
                line = raw.strip()
                if line.startswith("#["):
                    r = RENAME.search(line)
                    pending = r.group(1) if r else pending
                    continue
                if line.startswith("//"):
                    continue
                f = FIELD.match(line)
                if f and not f.group(2).lstrip().startswith("Option<"):
                    field = f.group(1)
                    names.add(pending or (camel(field) if all_camel else field))
                if f:
                    pending = None
            if names:
                # EVERY definition of the name, not the first the walk happens to
                # reach. The system monitor declares a `Process` in its Tauri
                # crate and another in its core crate; taking one of them at
                # random reported four fields missing from an answer that had
                # eight of nine, which cannot be true of a single struct.
                out.setdefault(m.group(1), []).append(names)
    return out


def literal_at(text: str, i: int, mod) -> set[str] | None:
    """Keys of the object literal starting at `i`, over at most one `[`."""
    n = len(text)
    seen_bracket = False
    while i < n and (text[i] in " \n\t" or (text[i] == "[" and not seen_bracket)):
        seen_bracket = seen_bracket or text[i] == "["
        i += 1
    if i >= n or text[i] != "{":
        return None
    return js_keys(mod.balanced_body(text, i))


def named_literals(text: str, mod) -> dict[str, set[str]]:
    """Same-file constants initialised with an object literal, by name.

    Eleven of the nineteen answers in this directory hand back a named constant
    rather than an inline literal - `ROWS`, `PROFILES`, `STATE` - and skipping
    them left the check reading a quarter of what it was written for. The ones it
    still cannot follow are arrays of HELPER CALLS (`[entry("notes.md", ...)]`),
    where the keys live in a function this has no business interpreting.
    """
    out: dict[str, set[str]] = {}
    for m in DECL.finditer(text):
        keys = literal_at(text, m.end(), mod)
        if keys:
            out[m.group(1)] = keys
    return out


def answered_keys(text: str, after: int, mod) -> set[str] | None:
    """The keys of the object this branch resolves, or None if it is not literal."""
    # Bounded by the NEXT branch, not by a character count. A branch that
    # REJECTS - which is the one every refusal fixture is built around - has no
    # answer of its own, and a plain look-ahead found the `Promise.resolve` of
    # the branch below it and measured that against this command's struct. The
    # arithmetic gave it away again: three of five fields present with five
    # named missing cannot be true of one answer.
    nxt = text.find("cmd ===", after)
    limit = nxt if nxt > 0 else len(text)
    resolve = text.find("Promise.resolve", after)
    if resolve < 0 or resolve > limit or resolve - after > 500:
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
    keys = literal_at(text, i, mod)
    if keys is not None:
        return keys
    # A named constant declared in the same file, which is how most of these
    # fixtures hold a richer answer.
    while i < n and text[i] in " \n\t[":
        i += 1
    name = re.match(r"[A-Za-z_$][\w$]*", text[i:])
    if not name:
        return None
    return named_literals(text, mod).get(name.group(0))


def main() -> int:
    mod = shape_module()
    returns = mod.rust_return_types(ROOT)

    # command -> [(app, struct)], every app that declares it. A fixture does not
    # say which app it drives, so an answer passes if it satisfies ANY declaring
    # app - conservative where two apps share a command name.
    by_command: dict[str, list[tuple[str, str]]] = {}
    for app, cmds in returns.items():
        for command, struct in cmds.items():
            by_command.setdefault(command, []).append((app, struct))

    per_app: dict[str, dict[str, list[set[str]]]] = {}

    def wanted(app: str, struct: str) -> list[set[str]]:
        if app not in per_app:
            # The whole app, not just `src-tauri`: the screenshot editor keeps
            # `OutputDto` in its `core` crate and the check found nothing for it.
            # Still narrow enough for the reason the scope exists - two apps with
            # a `Process` of their own no longer answer for each other.
            src = ROOT / "apps" / app
            per_app[app] = required_fields(src, mod) if src.is_dir() else {}
        return per_app[app].get(struct, [])

    checked = 0
    unread = 0
    carried = 0
    bad: list[str] = []
    hosts = sorted(HOSTS.glob("*.js"))
    if not hosts:
        print("check-fixture-answers-whole: no host fixtures found, so the scan is pointed wrong")
        return 1
    for path in hosts:
        text = path.read_text(encoding="utf-8", errors="replace")
        for branch in BRANCH.finditer(text):
            command = branch.group(1)
            declared = by_command.get(command)
            if not declared:
                continue
            wants = [(a, st, w) for a, st in declared for w in wanted(a, st) if w]
            if not wants:
                continue
            got = answered_keys(text, branch.end(), mod)
            if got is None:
                unread += 1
                continue
            checked += 1
            shortfalls = [(st, sorted(w - got)) for _, st, w in wants]
            if any(not miss for _, miss in shortfalls):
                continue
            struct, missing = min(shortfalls, key=lambda x: len(x[1]))
            want = next(w for _, st, w in wants if st == struct)
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
        " The command's return type comes from the same resolver `check-invoke-shape`"
        " uses; the field names are the WIRE ones, with `serde` renames applied."
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
