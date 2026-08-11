# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that nothing started from a Tauri setup hook assumes a tokio runtime.

Tauri's `setup` hook runs on the main thread, before and outside the async
runtime the rest of the process uses. Tokio's own entry points do not fail
politely there: `tokio::spawn` and `tokio::net::UnixListener::bind` panic with
"there is no reactor running", which in a setup hook means a main-thread panic
during startup.

That is what the desktop shell did, on every single start, for as long as the
launch socket has existed. `spawn_launch_service()` bound with tokio's listener,
so the socket the portal and the file manager were both moved onto never bound
once - and the function had a perfectly good error arm underneath, which `bind`
never reached because it panicked a line earlier. Nothing caught it: the shell's
379 tests pass, the source checks pass, and the panic only appears if you start
the shell and read its log.

The scope below is a FILE NAME, and that is a dependency worth naming. The marker
that actually matters is `.setup(`, and it lives in `lib.rs` in all seven apps
that have one - Tauri's own template puts the builder there, and nothing in the
tree deviates (measured 11 Aug: zero `.setup(` in any `main.rs` or elsewhere under
`src-tauri/src`). So the glob is complete today and silently would not be for an
app that built elsewhere. The sibling `check-executor-gate.py` had the same shape
and one file already breaking it, which is why this note exists rather than a
comfortable silence.

What this checks: for every `apps/*/src-tauri/src/lib.rs`, the functions called
from inside `.setup(...)`, and whether any of them is a NON-ASYNC function whose
body reaches for tokio directly. The correct spelling in this tree is
`tauri::async_runtime::spawn`, which is what the three sibling IPC services
(clipboard, intent, search) use.

Scoped to the setup path on purpose. A sync helper that calls `tokio::spawn` is
perfectly fine when its caller is an async command, and there is one of those in
Settings; widening this to every sync function would flag it and earn an
exception list, which is how a check turns into a formality.

What it does NOT cover:

  * transitive depth beyond the functions setup names directly. The bug was at
    depth one, and a full call graph in a regex is a worse lie than a shallow
    check that says how deep it went.
  * `block_on` inside setup, which deadlocks rather than panics. Different
    failure, not seen in this tree.
  * whether the service actually works once started. Only running it shows that,
    which is how this one was found.

Shown to fail before being trusted: restoring `tokio::net::UnixListener` in
`launch_service::bind` makes it name that function.
"""

import re
import sys
from pathlib import Path

# A tree to check may be passed in, which is what lets this gate's own test drive
# it against fixtures; the sibling gates take the same argument.
ROOT = (
    Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else Path(__file__).resolve().parents[2]
)

# The reaches that need a reactor already running.
NEEDS_RUNTIME = (
    re.compile(r"(?<!async_runtime::)\btokio::spawn\s*\("),
    re.compile(r"\btokio::net::[A-Za-z]+::bind\s*\("),
)


def setup_body(lib: str) -> str:
    """The text of the `.setup(...)` closure, by brace balance from `.setup(`."""
    start = lib.find(".setup(")
    if start == -1:
        return ""
    depth = 0
    for i in range(start + len(".setup"), len(lib)):
        if lib[i] == "(":
            depth += 1
        elif lib[i] == ")":
            depth -= 1
            if depth == 0:
                return lib[start:i]
    return lib[start:]


def called_names(body: str) -> set[str]:
    """Function names called in the setup body, last path segment only."""
    out = set()
    for m in re.finditer(r"\b(?:([a-z_][a-z0-9_]*)::)*([a-z_][a-z0-9_]*)\s*\(", body):
        out.add(m.group(2))
    return out


def fn_bodies(app_src: Path) -> dict[str, tuple[bool, str, Path]]:
    """Every function in the app: name to (is_async, body, file)."""
    out: dict[str, tuple[bool, str, Path]] = {}
    for path in app_src.rglob("*.rs"):
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r"\n(pub(?:\([^)]*\))?\s+)?(async\s+)?fn\s+([a-z_][a-z0-9_]*)", text):
            name, is_async = m.group(3), m.group(2) is not None
            brace = text.find("{", m.end())
            if brace == -1:
                continue
            depth, end = 0, len(text)
            for i in range(brace, len(text)):
                if text[i] == "{":
                    depth += 1
                elif text[i] == "}":
                    depth -= 1
                    if depth == 0:
                        end = i
                        break
            out[name] = (is_async, text[brace:end], path)
    return out


def main() -> int:
    findings: list[str] = []
    checked = 0
    for lib in sorted((ROOT / "apps").glob("*/src-tauri/src/lib.rs")):
        app = lib.relative_to(ROOT).parts[1]
        body = setup_body(lib.read_text(encoding="utf-8", errors="replace"))
        if not body:
            continue
        # The hook's own body, before anything it calls. This used to check only
        # the functions setup NAMES, so a `tokio::spawn` written straight into the
        # closure - the shortest way to write this defect, needing no helper at
        # all - passed. The instances that prompted the check happened to be
        # helpers, and it was built to their shape. Measured on 11 August: no
        # setup body reaches tokio today, so this is for the next one.
        checked += 1
        for pattern in NEEDS_RUNTIME:
            if pattern.search(body):
                findings.append(
                    f"{lib.relative_to(ROOT)}: {app}'s setup hook reaches for tokio "
                    f"directly in its own body, which is not inside a runtime - that "
                    f"panics on 'there is no reactor running' at startup. Use "
                    f"`tauri::async_runtime::spawn`."
                )
                break

        fns = fn_bodies(lib.parent)
        for name in sorted(called_names(body)):
            found = fns.get(name)
            if not found:
                continue
            is_async, fn_body, path = found
            if is_async:
                continue
            checked += 1
            for pattern in NEEDS_RUNTIME:
                if pattern.search(fn_body):
                    findings.append(
                        f"{path.relative_to(ROOT)}: `{name}` is called from {app}'s "
                        f"setup hook, which is not inside a runtime, and reaches for "
                        f"tokio directly - that panics on 'there is no reactor "
                        f"running' at startup. Use `tauri::async_runtime::spawn`."
                    )
                    break

    print(
        f"{checked} setup hook body/bodies and the functions they call checked "
        f"for a tokio reach that needs a runtime already running. Depth one "
        f"only: the hook itself and what it names directly, not what those go "
        f"on to call."
    )
    if findings:
        print("\nstarted from setup, panics without a runtime:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
