# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""A D-Bus call must pass as many arguments as the member it dials declares.

`check-dbus-members-exist` asks whether the member exists. This asks the next
question, which turns out to be the one that bites: zbus matches a method by name
AND signature, so a caller one argument behind a served method is answered with an
error rather than served - and every job producer in this tree ignores that error
on purpose, because a progress report must never break the work it reports on.

WHAT WENT WRONG, on 7 September. `a5e8e35ab` gave the job server's `Register` a
ninth argument (the items a job will work through). It updated the file manager,
which hand-rolls its proxy, and not `contracts/notification-proto`'s shared client
nor either of the two producers using it. Measured against the real daemon on a
private session bus:

    eight arguments: Signature mismatch: got `(ssstbbbs)`, expected `(ssstbbbsas)`
    nine arguments:  method return ... uint64 1

So forage's builds and the model downloads in Settings registered nothing at all,
on the surface built to show exactly them, and nothing said so. A best-effort
contract cannot report its own breakage; that is what makes this worth a check
rather than a test.

WHAT IT COMPARES. Every member SERVED by a `#[zbus::interface]` impl in this tree
is recorded under its WIRE name - zbus renames `set_state` to `SetState` unless
`#[zbus(name = "...")]` says otherwise - along with how many arguments a caller
sends it. Arguments zbus injects are not sent by anybody and are excluded:
`#[zbus(header)]`, `#[zbus(connection)]`, `#[zbus(object_server)]`,
`#[zbus(signal_context)]` and their kin. Then two caller shapes are counted:

  * `proxy.call("Member", &(a, b, c))` and its `call_method` / turbofish variants
  * a `#[proxy]` trait's `async fn member(&self, ...)`

WHEN IT STAYS QUIET, and each of these is deliberate:

  * a member name this tree does not serve. Most `.call(` sites in the tree dial
    BlueZ, NetworkManager or logind, which we neither own nor can read
  * a member name served by more than one of our interfaces with different arity.
    The call site names an interface through a const, not a literal, so resolving
    which one is a variable hop; an ambiguous name is skipped rather than guessed
  * a tuple that is not a literal - `&args`, `&build_args()`. Nothing to count
  * signals and properties on a proxy trait. Their shapes are not a method call
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
SKIP = {"target", "node_modules", "mkosi.builddir", ".git", ".svelte-kit", "build", "dist"}

INTERFACE = re.compile(r"#\[(?:zbus::)?interface\b")
PROXY = re.compile(r"#\[(?:zbus::)?proxy\b")
# The interface a block serves or a proxy dials, out of its own attribute.
IFACE_NAME = re.compile(r'\bname\s*=\s*"([^"]+)"')
PROXY_IFACE = re.compile(r'\binterface\s*=\s*"([^"]+)"')
ANY_IFACE_LITERAL = re.compile(r'"((?:org|com|net)\.[A-Za-z0-9_.]+)"')
EXPLICIT = re.compile(r'#\[zbus\(name\s*=\s*"([^"]+)"\)\]')
FN = re.compile(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(")
# `#[zbus(header)] header: ...` and friends: injected by the bus, sent by nobody.
INJECTED = re.compile(r"#\[zbus\((?:header|connection|object_server|signal_context|signal_emitter)\)\]")
CALL = re.compile(r'\.call(?:_method)?(?:::<[^>]*>)?\(\s*"([A-Za-z_][A-Za-z0-9_]*)"\s*,\s*&\(')

KNOWN: dict[str, str] = {}


def sources() -> list[Path]:
    out = []
    for p in ROOT.rglob("*.rs"):
        if any(part in SKIP for part in p.parts):
            continue
        out.append(p)
    return sorted(out)


def wire_name(fn: str) -> str:
    """zbus's default rename: `set_state` becomes `SetState`."""
    return "".join(part[:1].upper() + part[1:] for part in fn.split("_") if part)


def strip_comments(text: str) -> str:
    """Line comments out of an argument list.

    Load-bearing rather than tidiness: the file manager's `Register` call carries
    a comment per argument explaining the flag, and two of those sentences contain
    a comma - so counting commas without this reads an eight-argument call as
    eleven and reports a mismatch that is not there. Found by running the check.
    """
    return "\n".join(line.split("//")[0] for line in text.split("\n"))


def split_top_level(text: str) -> list[str]:
    """Split on commas that are not inside brackets or a string.

    A lifetime is NOT a quote. `Header<'_>` opens a `'` that never closes, and
    treating it as one swallowed the rest of every parameter list that had one -
    which is why the first run of this check reported that half the daemons serve
    methods taking no arguments at all. Only `"` opens a string here; a char
    literal in a Rust parameter list or a call tuple does not occur.
    """
    text = strip_comments(text)
    out, depth, cur, quote = [], 0, "", None
    i = 0
    while i < len(text):
        c = text[i]
        if quote:
            if c == "\\":
                cur += text[i : i + 2]
                i += 2
                continue
            if c == quote:
                quote = None
        elif c == '"':
            quote = c
        elif c in "([{<":
            depth += 1
        elif c in ")]}>":
            depth -= 1
        elif c == "," and depth == 0:
            out.append(cur)
            cur = ""
            i += 1
            continue
        cur += c
        i += 1
    out.append(cur)
    return [s for s in (x.strip() for x in out) if s]


def balanced(text: str, start: int) -> str:
    """The contents of the `(`...`)` whose opening paren is at `start - 1`."""
    depth, quote = 1, None
    i = start
    while i < len(text):
        c = text[i]
        if quote:
            if c == "\\":
                i += 2
                continue
            if c == quote:
                quote = None
        elif c == '"':
            quote = c
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return text[start:i]
        i += 1
    return ""


def blocks(text: str, marker: re.Pattern[str], iface: re.Pattern[str]) -> list[tuple[str, str]]:
    """Each `impl`/`trait` body whose attribute matches, with its interface name.

    THE INTERFACE IS LOAD-BEARING, not decoration. The first cut matched on the
    member name alone and reported six mismatches that were not: this tree's
    portal daemon serves `org.freedesktop.impl.portal.FileChooser`, whose
    `OpenFile` takes the handle and app id the FRONTEND adds, while the plugin
    dials `org.freedesktop.portal.FileChooser`'s three-argument `OpenFile`. Same
    member name, two interfaces, two correct signatures.
    """
    out = []
    for m in marker.finditer(text):
        attr_end = text.find("]", m.end())
        attr = text[m.end() : attr_end] if attr_end > 0 else ""
        found = iface.search(attr)
        name = found.group(1) if found else ""
        brace = text.find("{", attr_end if attr_end > 0 else m.end())
        if brace < 0:
            continue
        depth = 0
        for i in range(brace, len(text)):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    out.append((name, text[brace : i + 1]))
                    break
    return out


def methods(body: str) -> dict[str, int]:
    """Wire name to the number of arguments a CALLER sends, for one block."""
    out: dict[str, int] = {}
    for m in FN.finditer(body):
        args = balanced(body, m.end())
        parts = split_top_level(args)
        # Drop the receiver and anything the bus injects.
        parts = [
            p
            for p in parts
            if not p.replace("&", "").replace("mut ", "").strip().startswith("self")
            and not INJECTED.search(p)
        ]
        before = body[: m.start()]
        rename = EXPLICIT.findall(before[-400:])
        name = rename[-1] if rename and "fn " not in before[before.rfind(rename[-1]) :] else wire_name(m.group(1))
        out[name] = len(parts)
    return out


def main() -> int:
    files = sources()
    # (interface, wire member) -> how many arguments a caller sends.
    served: dict[tuple[str, str], int] = {}
    for p in files:
        text = p.read_text(encoding="utf-8", errors="replace")
        if not INTERFACE.search(text):
            continue
        for iface, body in blocks(text, INTERFACE, IFACE_NAME):
            if not iface:
                continue
            for name, n in methods(body).items():
                served[(iface, name)] = n

    bad: list[str] = []
    checked = 0
    for p in files:
        rel = str(p.relative_to(ROOT))
        text = p.read_text(encoding="utf-8", errors="replace")

        # A `.call(` site names its interface through a const, not a literal, so
        # the interface is resolved from the literals the FILE carries. Exactly
        # one of them must serve this member; anything else is left alone.
        file_ifaces = set(ANY_IFACE_LITERAL.findall(text))
        for m in CALL.finditer(text):
            name = m.group(1)
            matches = [(i, n) for (i, mem), n in served.items() if mem == name and i in file_ifaces]
            if len(matches) != 1:
                continue
            sent = len(split_top_level(balanced(text, m.end())))
            checked += 1
            if sent != matches[0][1] and rel not in KNOWN:
                line = text[: m.start()].count("\n") + 1
                bad.append(
                    f"  - {rel}:{line}: `{name}` on {matches[0][0]} sent {sent},"
                    f" served takes {matches[0][1]}"
                )

        if PROXY.search(text):
            for iface, body in blocks(text, PROXY, PROXY_IFACE):
                for name, n in methods(body).items():
                    want = served.get((iface, name))
                    if want is None or "signal" in body[: body.find(name)][-200:]:
                        continue
                    checked += 1
                    if n != want and rel not in KNOWN:
                        bad.append(
                            f"  - {rel}: proxy `{name}` on {iface} declares {n},"
                            f" served takes {want}"
                        )

    print(
        f"{checked} D-Bus call site(s) compared against {len(served)} member(s) this tree"
        " serves. A member on an interface nothing here serves, an interface the caller's"
        " file never names, or an argument list that is not a literal tuple, is not counted."
    )
    if bad:
        print("\ncalls that do not match the member they dial:\n")
        print("\n".join(bad))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
