# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every `invoke(...)` call passes the arguments its Rust command declares.

The failure this exists for is silent. Tauri deserializes the payload object into
the command's parameters; a key the command does not declare is ignored, and a
parameter the caller never sent arrives as a deserialization error the frontend
usually swallows. So a renamed or mistyped argument does not crash - the command
runs with a default, or the call quietly fails, and the surface shows something
plausible and wrong. Nothing in the type system connects the two sides: the
frontend writes an object literal, the backend writes a function signature, and
they meet at a string.

Tauri's own convention is part of the trap. A snake_case parameter is reachable
from JavaScript as camelCase, so `app_id` and `appId` are both correct and
`app_ID` is not, and no tool says so.

What this compares:

  * every `#[tauri::command]` function's parameter names, skipping the injected
    ones (`State`, `AppHandle`, `Window`, `WebviewWindow`) that never come from
    the caller
  * every `invoke("name", { ... })` call's object keys

and reports keys the command does not declare, plus required parameters the call
omits. Both names are normalised to snake_case first, so the camelCase
convention is accepted rather than flagged.

The return direction is checked too, conservatively. For a call annotated
`invoke<T>("cmd")` where `T` resolves to a TypeScript interface in the same app
and the command returns a struct this can find, any field the interface declares
that the struct does not produce is reported: that field arrives `undefined` and
the surface renders a blank, a zero or a crash, with nothing thrown. The reverse
(a field the backend sends and the interface omits) is not a defect - the
frontend simply ignores it - so it is not reported.

Both sides are compared in snake_case, since `#[serde(rename_all = "camelCase")]`
is the house convention and both spellings mean the same field.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Parameters Tauri injects rather than reading from the payload.
INJECTED = re.compile(
    r"\b(State|AppHandle|Window|WebviewWindow|Manager|Runtime|Request|Response)\b"
)

# A command whose payload is a single struct the caller spreads, or which reads
# its arguments in a way this cannot see, would produce noise. None are known
# today; the set is here so one can be excused with a reason rather than by
# weakening the check for everyone.
EXCUSED: dict[str, str] = {}

# Return-shape mismatches that are real and belong to someone else's lane. Listed
# with a reason rather than skipped silently: the check still prints them, it just
# does not fail on them, so the debt stays visible and attributed. Keyed by
# command name; remove the entry when the owner fixes it and the gate holds it shut.
KNOWN_RETURN_MISMATCHES: dict[str, str] = {
    "ai_models_search_hf": (
        "the model picker's `Model` is the merged card shape the page builds from "
        "several sources; the Hugging Face hit is only one of them. Reworking it is "
        "arlen-ui's model-picker job, and its route is their live work."
    ),
    "store_search": (
        "install variants are arlen-ui's store design item; the card model changes "
        "with it."
    ),
    "store_outdated": (
        "the pending-updates surface is arlen-ui's; the backend `PendingUpdate` and "
        "the frontend one are different shapes that have not been reconciled yet."
    ),
}


def snake(name: str) -> str:
    """camelCase or snake_case to snake_case, so both spellings compare equal."""
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def rust_commands(root: Path) -> dict[str, dict[str, tuple[set[str], set[str]]]]:
    """Map app to command name to (all parameter names, required parameter names).

    Per app, not global: several apps define a `frontend_log`, and they do not
    agree on its parameters. A global map compares a call against whichever app
    was scanned last, which is how the first run of this check reported eight
    confident findings that were all the same mistake.
    """
    out: dict[str, dict[str, tuple[set[str], set[str]]]] = {}
    for path in root.glob("apps/*/src-tauri/src/**/*.rs"):
        app = path.relative_to(root).parts[1]
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(
            r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\(([^)]*)\)",
            text,
            re.S,
        ):
            name, params = m.group(1), m.group(2)
            all_p: set[str] = set()
            required: set[str] = set()
            for raw in split_params(params):
                if not raw.strip() or INJECTED.search(raw):
                    continue
                decl = raw.split(":", 1)
                if len(decl) != 2:
                    continue
                pname = decl[0].strip().lstrip("_").replace("r#", "")
                if not pname or not pname.isidentifier():
                    continue
                all_p.add(snake(pname))
                # `Option<T>` may be omitted; everything else must be sent.
                if "Option<" not in decl[1]:
                    required.add(snake(pname))
            out.setdefault(app, {})[name] = (all_p, required)
    return out


def split_params(params: str) -> list[str]:
    """Split a parameter list on commas that are not inside a generic or tuple."""
    depth, cur, out = 0, "", []
    for ch in params:
        if ch in "<([":
            depth += 1
        elif ch in ">)]":
            depth -= 1
        if ch == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += ch
    out.append(cur)
    return out


def invoke_calls(root: Path):
    """Yield (app, file, line, command, argument keys or None) for every call."""
    for base in (root / "apps",):
        for path in base.rglob("*"):
            if path.suffix not in (".ts", ".svelte") or not path.is_file():
                continue
            if "node_modules" in path.parts or ".svelte-kit" in path.parts:
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for m in re.finditer(r'invoke\s*(?:<[^>]*>)?\s*\(\s*"([a-z_0-9]+)"', text):
                cmd = m.group(1)
                line = text[: m.start()].count("\n") + 1
                keys = payload_keys(text, m.end())
                yield (
                    path.relative_to(root).parts[1],
                    path.relative_to(root),
                    line,
                    cmd,
                    keys,
                )


def payload_keys(text: str, pos: int) -> set[str] | None:
    """The payload object's top-level keys, `set()` for no payload, None if unreadable.

    Reads the object with a brace counter rather than a regex. The regex version
    stopped at the first `}`, which in practice is the one closing a `${...}` in a
    template literal, so every call whose argument interpolated anything looked
    like a call with no arguments at all - eight confident findings, all wrong.
    Telling "no payload" apart from "a payload I cannot read" is the whole point:
    the first is a finding, the second must be skipped.
    """
    i = pos
    while i < len(text) and text[i] in " \t\n\r":
        i += 1
    if i >= len(text):
        return None
    if text[i] == ")":
        return set()
    if text[i] != ",":
        return None
    i += 1
    while i < len(text) and text[i] in " \t\n\r":
        i += 1
    if i >= len(text) or text[i] != "{":
        # A variable or spread rather than a literal: nothing to compare.
        return None
    depth, start = 0, i
    while i < len(text):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return object_keys(text[start + 1 : i])
        i += 1
    return None


def object_keys(body: str) -> set[str] | None:
    """The top-level keys of an object body, or None if it cannot be read flatly.

    Splits on commas outside braces, brackets, parens and template literals, so a
    nested object or an interpolated string in a value does not break the key list.
    """
    if "..." in body:
        return None
    keys: set[str] = set()
    depth, in_tpl, cur = 0, False, ""
    i = 0
    while i < len(body):
        ch = body[i]
        if ch == "`":
            in_tpl = not in_tpl
        elif not in_tpl and ch in "{[(":
            depth += 1
        elif not in_tpl and ch in "}])":
            depth -= 1
        if ch == "," and depth == 0 and not in_tpl:
            keys.add(cur)
            cur = ""
            i += 1
            continue
        cur += ch
        i += 1
    parts = [p for p in [*keys, cur]]
    out: set[str] = set()
    for part in parts:
        part = part.strip()
        if not part:
            continue
        key = part.split(":", 1)[0].strip()
        if not key.isidentifier():
            return None
        out.add(snake(key))
    return out


# Rust primitives and containers that carry no named struct to compare against.
OPAQUE_RETURN = re.compile(
    r"^(String|bool|u\d+|i\d+|f\d+|usize|isize|char|serde_json::Value|Value|\(\))$"
)


def rust_return_types(root: Path) -> dict[str, dict[str, str]]:
    """Map app to command name to the bare name of the type it returns."""
    out: dict[str, dict[str, str]] = {}
    for path in root.glob("apps/*/src-tauri/src/**/*.rs"):
        app = path.relative_to(root).parts[1]
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(
            r"#\[tauri::command[^\]]*\]\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*\([^)]*\)"
            r"\s*->\s*([^{;]+)",
            text,
            re.S,
        ):
            out.setdefault(app, {})[m.group(1)] = bare_type(m.group(2))
    return out


def bare_type(ret: str) -> str:
    """Strip Result/Option/Vec wrappers down to the payload type's name."""
    t = ret.strip().rstrip("{").strip()
    for _ in range(6):
        m = re.match(r"^(?:Result|Option|Vec)\s*<\s*(.+)$", t)
        if not m:
            break
        inner = m.group(1)
        depth, cut = 0, len(inner)
        for i, ch in enumerate(inner):
            if ch == "<":
                depth += 1
            elif ch == ">":
                if depth == 0:
                    cut = i
                    break
                depth -= 1
            elif ch == "," and depth == 0:
                cut = i
                break
        t = inner[:cut].strip()
    return t.split("::")[-1].strip()


def rust_struct_fields(root: Path) -> dict[str, set[str]]:
    """Map struct name to its field names, for every struct in the tree.

    Field names only, in snake_case. A struct name defined twice is dropped
    rather than guessed at: comparing against the wrong one is how the argument
    half produced its first round of false findings.
    """
    fields: dict[str, set[str]] = {}
    seen_twice: set[str] = set()
    for path in root.rglob("*.rs"):
        if "target" in path.parts or "node_modules" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r"struct\s+(\w+)\s*\{([^}]*)\}", text, re.S):
            name, body = m.group(1), m.group(2)
            got = set()
            for line in body.splitlines():
                line = line.strip()
                if not line or line.startswith(("#", "//", "///")):
                    continue
                fm = re.match(r"(?:pub\s+)?(\w+)\s*:", line)
                if fm:
                    got.add(snake(fm.group(1)))
            if name in fields and fields[name] != got:
                seen_twice.add(name)
            fields[name] = got
    for name in seen_twice:
        fields.pop(name, None)
    return fields


def ts_interfaces(root: Path) -> dict[str, dict[str, set[str]]]:
    """Map app to interface name to its declared field names, ambiguity dropped."""
    out: dict[str, dict[str, set[str]]] = {}
    twice: dict[str, set[str]] = {}
    for path in (root / "apps").rglob("*"):
        if path.suffix not in (".ts", ".svelte") or not path.is_file():
            continue
        if "node_modules" in path.parts or ".svelte-kit" in path.parts:
            continue
        app = path.relative_to(root).parts[1]
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r"interface\s+(\w+)\s*\{([^}]*)\}", text, re.S):
            name, body = m.group(1), m.group(2)
            got = set()
            for line in body.splitlines():
                line = line.strip()
                if not line or line.startswith(("//", "/*", "*", "///")):
                    continue
                # Only fields the interface declares as REQUIRED. A `field?:` is
                # the frontend saying it already handles absence - the print queue's
                # `progress?` is exactly that, and reporting it would be noise.
                fm = re.match(r"(\w+)\s*:", line)
                if fm:
                    got.add(snake(fm.group(1)))
            bucket = out.setdefault(app, {})
            if name in bucket and bucket[name] != got:
                twice.setdefault(app, set()).add(name)
            bucket[name] = got
    for app, names in twice.items():
        for n in names:
            out[app].pop(n, None)
    return out


def annotated_calls(root: Path):
    """Yield (app, file, line, type name, command) for `invoke<T>("cmd")` calls."""
    for path in (root / "apps").rglob("*"):
        if path.suffix not in (".ts", ".svelte") or not path.is_file():
            continue
        if "node_modules" in path.parts or ".svelte-kit" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(
            r'invoke\s*<\s*(\w+)(?:\[\])?\s*>\s*\(\s*"([a-z_0-9]+)"', text
        ):
            yield (
                path.relative_to(root).parts[1],
                path.relative_to(root),
                text[: m.start()].count("\n") + 1,
                m.group(1),
                m.group(2),
            )


def check_returns(root: Path) -> tuple[int, list[str], list[str]]:
    """Report interface fields the command's return struct does not produce."""
    returns = rust_return_types(root)
    structs = rust_struct_fields(root)
    interfaces = ts_interfaces(root)
    problems: list[str] = []
    known: list[str] = []
    checked = 0
    for app, path, line, tsname, cmd in annotated_calls(root):
        rust_name = returns.get(app, {}).get(cmd)
        if not rust_name or OPAQUE_RETURN.match(rust_name):
            continue
        produced = structs.get(rust_name)
        declared = interfaces.get(app, {}).get(tsname)
        if produced is None or declared is None or not produced or not declared:
            continue
        checked += 1
        missing = declared - produced
        if not missing:
            continue
        text = (
            f"{path}:{line}: `{cmd}` returns `{rust_name}`, which does not produce "
            f"{sorted(missing)}; `{tsname}` declares them, so they arrive undefined"
        )
        if cmd in KNOWN_RETURN_MISMATCHES:
            known.append(f"{text}\n      routed: {KNOWN_RETURN_MISMATCHES[cmd]}")
        else:
            problems.append(text)
    return checked, problems, known


def main() -> int:
    # A directory argument scans that tree instead of the repo's. Only the
    # fixture runner passes one; CI passes nothing. This check has produced false
    # findings twice (a command map shared across apps, and a payload reader that
    # stopped at the first `}` of a `${...}`), so it has to be runnable against
    # inputs picked to fool it.
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT

    commands = rust_commands(root)
    if not commands:
        print("found no #[tauri::command] functions; the check needs updating")
        return 2

    problems = []
    checked = 0
    total = sum(len(v) for v in commands.values())
    for app, path, line, cmd, keys in invoke_calls(root):
        own = commands.get(app, {})
        if cmd not in own or cmd in EXCUSED or keys is None:
            continue
        declared, required = own[cmd]
        checked += 1
        extra = keys - declared
        missing = required - keys
        if extra:
            problems.append(
                f"{path}:{line}: `{cmd}` is passed {sorted(extra)}, which it does not "
                f"declare; Tauri drops the key and the command runs without it"
            )
        if missing:
            problems.append(
                f"{path}:{line}: `{cmd}` needs {sorted(missing)}, which this call does "
                f"not pass; the command fails to deserialize its arguments"
            )

    ret_checked, ret_problems, ret_known = check_returns(root)
    problems.extend(ret_problems)

    print(
        f"{checked} invoke call(s) checked against {total} command(s); "
        f"{ret_checked} annotated return type(s) compared"
    )
    if problems:
        print("\nshapes that do not match the command on the other side:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    if ret_known:
        print("\nknown return mismatches, routed to their owners:\n")
        for k in ret_known:
            print(f"  - {k}")
    print("\nevery call passes the arguments its command declares")
    return 0


if __name__ == "__main__":
    sys.exit(main())
