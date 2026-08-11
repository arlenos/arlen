# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every Tauri app contains its WebKit web process.

The decision was to turn the WebKitGTK sandbox on app by app and grant what
breaks, rather than keep an exemption list that decays into everything being
exempt. Twelve apps had it and the thirteenth did not, and nothing said so - the
only way to know was to grep. A fourteenth would have been the same, which is why
this exists rather than another round of grepping.

What it checks: every `apps/*/src-tauri/src/main.rs` sets `WEBKIT_FORCE_SANDBOX`
before the app starts.

**A CONFINED launch declines it, and that is not a loophole in this rule - it is
the rule doing its job in a place where the nesting cannot hold.** `arlen-run`
passes `WEBKIT_FORCE_SANDBOX=0`, which each app respects because it sets the
variable only when the environment carries none. It has to: WebKit's sandbox needs
a nested user namespace, and the app seccomp filter denies one, so with the inner
sandbox forced the window opens and the webview paints nothing.

**What that costs, stated because a deliberate choice is not a free one.** The two
sandboxes bound different things. WebKit's bounds a compromised RENDERER away from
the app's own files; bwrap bounds the APP away from the system. Declining the inner
one means a compromised renderer holds the app's entire grant - so under
confinement the permission profile is not paperwork, it is the only boundary left
around a renderer that has been taken over. Review a confined app's profile with
that in mind.

What it does NOT check, and this is the part worth reading before trusting a pass:

  * That the variable is set BEFORE GTK or WebKit initialises. It must be, or
    WebKit aborts the process with "Sandboxing cannot be changed after
    subprocesses were spawned" - which is loud, and a crash on first launch is
    not a silent failure, so it is left to the runtime rather than parsed here.
  * That the sandbox actually holds. This reads source; only a running app can
    show a contained renderer. `probe-webview-sandbox.sh` next to this file does
    that part - it starts an app under Xvfb and compares the web process's mount
    and user namespaces against the app's. Every app has now been watched holding
    it, the desktop shell included - that one needs a compositor rather than a
    bare X server, which is what the probe's `PROBE_WAYLAND=1` mode is for.
    Watching one is worth more than this file's whole pass, because
    the knowledge app read as NOT CONTAINED on the first watch and the cause was a
    binary older than the line that asks for containment - which this check, being
    a reader of source, cannot see and would never have caught.
  * Anything about the webviews the desktop shell creates for its own surfaces
    beyond its main process, or about the modules runtime's iframes, which are
    contained by a different mechanism (CSP + the broker).

So a pass here means "no app forgot to ask for containment", not "the renderers
are contained".
"""

import re
import sys
from pathlib import Path

# Takes the tree as an argument so the rule can be shown to fail on a planted
# app. Interleaved with the confinement work that changed this check's subject:
# the fixture was obvious while the defect was fresh.
ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

# An app that genuinely cannot run sandboxed belongs here with a reason and an
# owner, not silently missing from the tree. Empty is the goal and the current
# state: the adjudication was that an app which cannot work sandboxed gets said
# out loud and decided, rather than quietly exempted.
EXCUSED: dict[str, str] = {}

VAR = "WEBKIT_FORCE_SANDBOX"


def main() -> int:
    apps = sorted(p for p in (ROOT / "apps").glob("*/src-tauri/src/main.rs"))
    if not apps:
        print("no Tauri app entry points found; the layout moved and this check did not")
        return 1

    missing: list[str] = []
    excused: list[str] = []
    for path in apps:
        app = path.relative_to(ROOT).parts[1]
        text = path.read_text(encoding="utf-8", errors="replace")
        if re.search(rf'"{VAR}"', text):
            continue
        if app in EXCUSED:
            excused.append(f"{app}: {EXCUSED[app]}")
        else:
            missing.append(f"{path.relative_to(ROOT)}: {app} never sets {VAR}")

    print(
        f"{len(apps)} Tauri app(s) checked for {VAR}; "
        f"{len(missing)} without it, {len(excused)} excused. "
        "Source only - a pass means no app forgot to ask for containment, not "
        "that a renderer was observed contained."
    )
    if excused:
        print("\nrunning uncontained, with a reason:\n")
        for e in excused:
            print(f"  - {e}")
    if missing:
        print(
            "\napps whose WebKit renderer is not contained:\n\n"
            + "\n".join(f"  - {m}" for m in missing)
            + "\n\nSet it in main() before the app starts, next to the comment "
              "saying what that app's renderer is exposed to. If the app cannot "
              "work sandboxed, say why in EXCUSED so the decision is visible."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
