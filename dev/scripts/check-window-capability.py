#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every window a Tauri app opens must be covered by one of its capabilities.

A Tauri capability names the windows it applies to. A window label that appears in
no capability gets NOTHING: its webview is refused every gated command, including
`event|listen`, which is how a frontend learns anything happened at all.

Found on a booted image, in 77 log lines nobody was reading:

    arlen-shell: [frontend] unhandled rejection: Command plugin:event|listen not allowed by ACL

The shell opens `main`, `waypointer` and `consent`; its one capability listed
`main`. So the consent dialog could not subscribe to the event announcing a
consent request, and had never worked outside a dev session - silently, because
the refusal goes to a log and the window just sits there.

The rule: for each app under `apps/*/src-tauri`, every window label the Rust
creates or the config declares must appear in some capability's `windows` list
(or be covered by a `*` entry).

Labels are read from the two places Tauri takes them: `tauri.conf.json`'s
`app.windows[].label` (absent label means `main`, which is Tauri's own default),
and `WebviewWindowBuilder::new(app, <label>, ...)` in `src-tauri/src`, including
the common spelling where the label is a `const LABEL: &str` in the same file.

NOT covered: a label built at run time from a variable or `format!`, which this
cannot read. Those exist (per-document windows, say), and a capability for them
needs a glob the author writes deliberately; this catches the fixed labels, which
is what the shell's three were.

Shown to fail before being trusted: `dev/scripts/test-check-window-capability.mjs`.

Usage: check-window-capability.py [repo-root]
"""

import json
import re
import sys
from pathlib import Path

BUILDER = re.compile(r"WebviewWindowBuilder::new\(\s*[^,]+,\s*([A-Za-z_\"][^,]*?),")
CONST_LABEL = re.compile(r'const\s+([A-Z_][A-Z0-9_]*)\s*:\s*&str\s*=\s*"([^"]+)"')


def declared_labels(app: Path):
    """Window labels from tauri.conf.json, with Tauri's `main` default applied."""
    conf = app / "tauri.conf.json"
    if not conf.is_file():
        return set()
    try:
        data = json.loads(conf.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return set()
    windows = data.get("app", {}).get("windows", [])
    return {w.get("label") or "main" for w in windows}


def built_labels(app: Path):
    """Window labels the Rust opens, for the spellings that can be read statically."""
    labels = set()
    for path in (app / "src").rglob("*.rs"):
        try:
            text = path.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        consts = dict((m.group(1), m.group(2)) for m in CONST_LABEL.finditer(text))
        for m in BUILDER.finditer(text):
            arg = m.group(1).strip()
            if arg.startswith('"') and arg.endswith('"'):
                labels.add(arg[1:-1])
            elif arg in consts:
                labels.add(consts[arg])
            # else: a run-time label; see the module note.
    return labels


def covered_labels(app: Path):
    """Every window label the app's capabilities apply to."""
    covered = set()
    for path in sorted((app / "capabilities").glob("*.json")):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        for entry in data.get("windows", []):
            covered.add(entry)
    return covered


def uncovered(root: Path):
    """(app, sorted labels) for every app with a window no capability covers,
    plus how much was examined.

    The count is returned rather than kept local because the success message used
    to be a bare sentence. A gate that says "every window is covered" after
    finding no apps at all says exactly the same thing as one that checked
    fourteen - and this file's own subject, a window silently refused every gated
    command, is the same defect one layer down: a failure that looks like
    nothing happening. Two other checks were caught under-reading their subject
    on 11 Aug, both times because the number they printed could be compared with
    the size of the tree. This one can now be compared too.
    """
    findings = []
    apps = 0
    labels = 0
    for app in sorted(root.glob("apps/*/src-tauri")):
        if not (app / "capabilities").is_dir():
            continue
        apps += 1
        covered = covered_labels(app)
        wanted = declared_labels(app) | built_labels(app)
        labels += len(wanted)
        if "*" in covered:
            continue
        missing = sorted(wanted - covered)
        if missing:
            findings.append((str(app.relative_to(root)), missing))
    return findings, apps, labels


def main() -> int:
    root = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[2])
    findings, apps, labels = uncovered(root)
    if not findings:
        print(
            f"OK: {labels} window label(s) across {apps} app(s), each covered by a capability"
        )
        return 0
    print(f"WINDOW WITH NO CAPABILITY: {len(findings)} app(s)", file=sys.stderr)
    for app, missing in findings:
        print(f"  {app}: {', '.join(missing)}", file=sys.stderr)
    print(
        "  A window in no capability is refused every gated command, `event|listen`\n"
        "  included, so its frontend never hears that anything happened. The window\n"
        "  still opens, which is why this reads as a dead surface rather than an\n"
        "  error. Add the label to a capability's `windows`, or say in that file why\n"
        "  the window is meant to have nothing.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
