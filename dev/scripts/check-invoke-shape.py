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

Deliberately not checked here: the RETURN shape. That needs the Rust struct and
the TypeScript interface resolved and compared field by field, which is a
different and larger job; the return-shape mismatch that motivated this
(`store_trust_signals`) is named in the report rather than caught here.
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


def main() -> int:
    commands = rust_commands(ROOT)
    if not commands:
        print("found no #[tauri::command] functions; the check needs updating")
        return 2

    problems = []
    checked = 0
    total = sum(len(v) for v in commands.values())
    for app, path, line, cmd, keys in invoke_calls(ROOT):
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

    print(f"{checked} invoke call(s) checked against {total} command(s)")
    if problems:
        print("\narguments that do not match the command they are sent to:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("every call passes the arguments its command declares")
    return 0


if __name__ == "__main__":
    sys.exit(main())
