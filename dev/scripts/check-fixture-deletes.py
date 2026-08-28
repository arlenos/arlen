#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that no control deletes a directory it did not mint.

WHY THIS EXISTS, and it is the only check here written after the damage rather than
before it. On 27 August a control in `dev/scripts` passed the REPOSITORY ROOT to a
cleanup helper ending in `rmSync(dir, { recursive: true, force: true })`. It deleted
most of the working tree, `.git` included, and stopped only because one cache
directory was unwritable. Everything committed survived on the remote; one commit
made in the six minutes between the last push and the delete did not.

The pattern that allowed it is the ordinary one in every control here: a helper
takes a path as a PARAMETER and one caller passes a path it did not create. Care is
not the fix - care was already being applied, by someone who had written the helper
ten minutes earlier. The fix is that the delete refuses a path it has no record of
creating, which is what `lib/fixture.mjs` does.

So: a recursive delete in a control must come from that helper. A direct
`rmSync(x, { recursive: true })` is what this refuses.

THE LIST IS THE HONEST PART. Every control written before the helper existed still
deletes directly, and rewriting all of them at once is a worse risk than the one
being closed. They are named in MIGRATED_LATER, which MAY SHRINK AND MAY NOT GROW: a
new control cannot introduce an unguarded delete, which is where the danger actually
was - the delete that ran was in a file being written that evening.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)
SCRIPTS = ROOT / "dev/scripts"
HELPER = "lib/fixture.mjs"

# A recursive delete, in the shapes node offers. `force` is not part of the match:
# it decides whether a missing path is an error, not how much is removed.
DELETE = re.compile(
    r"""(?:rmSync|rm|rmdirSync)\s*\(         # the call
        [^;]*?                                # its arguments, up to the statement end
        recursive\s*:\s*true""",
    re.VERBOSE | re.DOTALL,
)

# A recursive remove reached through a shell, which takes just as much.
#
# Anchored on the CALL and not on the string. The first cut matched any quoted
# `rm -rf`, which meant a comment explaining this rule reported the file it was
# written in - measured on this check's own control. A prose mention is not a
# delete; an argument to `exec`/`spawn` is.
SHELL_DELETE = re.compile(
    r"""(?:exec|spawn)[A-Za-z]*\s*\(     # the call that reaches a shell
        [^)]*?                            # its arguments so far
        ["'`]\s*rm\s+-[a-zA-Z]*[rR]""",
    re.VERBOSE,
)


def without_strings(text: str) -> str:
    """The source with every string literal blanked out.

    A control that TESTS this check has to write a bad delete into a fixture, and a
    scanner that cannot tell code from a quoted description of code reports the
    control as the offender - which is exactly what happened the first time this
    ran. The same technique the Cypher token scanner in the knowledge daemon uses,
    for the same reason.

    Quotes are replaced rather than removed so every offset still lines up, and an
    escaped quote inside a literal does not end it.
    """
    out = []
    quote = None
    escaped = False
    for c in text:
        if quote:
            out.append(" " if c != "\n" else "\n")
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == quote:
                quote = None
            continue
        if c in "\"'`":
            quote = c
            out.append(" ")
            continue
        out.append(c)
    return "".join(out)


def controls() -> list[Path]:
    """Every control script, which is what may hold a fixture delete."""
    return sorted(p for p in SCRIPTS.glob("*.mjs") if p.name.startswith("test-"))


def offenders(code: str, raw: str) -> list[str]:
    """The unguarded deletes in one file, as the snippet that matched.

    `code` has string literals blanked; `raw` does not. See the call site for why
    the two rules are given different views of the same file.
    """
    found = []
    for m in DELETE.finditer(code):
        found.append(" ".join(m.group(0).split())[:80])
    for m in SHELL_DELETE.finditer(raw):
        found.append(" ".join(m.group(0).split())[:80])
    return found


# Controls that predate `lib/fixture.mjs` and still delete directly. Written from a
# measurement rather than by hand: the count is what it is, and every one of them
# takes its path from a variable in the same function, which is why they have not
# eaten anything. MAY SHRINK, MAY NOT GROW.
MIGRATED_LATER: set[str] = {
    "test-build-image-trap.mjs",
    "test-check-announced-refusal.mjs",
    "test-check-app-capabilities.mjs",
    "test-check-app-locale-used.mjs",
    "test-check-app-names-agree.mjs",
    "test-check-app-profiles.mjs",
    "test-check-apps-on-image.mjs",
    "test-check-binary-names.mjs",
    "test-check-bus-declarations.mjs",
    "test-check-bus-names-covered.mjs",
    "test-check-bus-path-in-source.mjs",
    "test-check-bus-socket-pins.mjs",
    "test-check-calendar-has-no-mail-path.mjs",
    "test-check-catalog-duplicates.mjs",
    "test-check-comment-paths.mjs",
    "test-check-components-rendered.mjs",
    "test-check-consent-input-ordering.mjs",
    "test-check-crate-coverage.mjs",
    "test-check-cypher-groups.mjs",
    "test-check-daemon-stop.mjs",
    "test-check-dbus-activation.mjs",
    "test-check-dbus-callers.mjs",
    "test-check-dbus-members-exist.mjs",
    "test-check-dbus-method-names.mjs",
    "test-check-default-handlers.mjs",
    "test-check-default-then-write.mjs",
    "test-check-dependency-direction.mjs",
    "test-check-desktop-entries.mjs",
    "test-check-dev-ports.mjs",
    "test-check-dev-prefix-admission.mjs",
    "test-check-emitters-declared.mjs",
    "test-check-executor-gate.mjs",
    "test-check-fixtures.mjs",
    "test-check-gate-drift.mjs",
    "test-check-gates-registered.mjs",
    "test-check-grant-visibility.mjs",
    "test-check-granted-and-used.mjs",
    "test-check-graph-clock.mjs",
    "test-check-graph-columns.mjs",
    "test-check-greetd-config.mjs",
    "test-check-help-citations.mjs",
    "test-check-image-contents.mjs",
    "test-check-image-writes.mjs",
    "test-check-inert-switches.mjs",
    "test-check-invoke-scope.mjs",
    "test-check-invoke-shape.mjs",
    "test-check-invokes.mjs",
    "test-check-kit-defaults.mjs",
    "test-check-knowledge-socket.mjs",
    "test-check-linked-libraries.mjs",
    "test-check-locale-adopted.mjs",
    "test-check-lockfiles-current.mjs",
    "test-check-log-filters.mjs",
    "test-check-menu-labels-translated.mjs",
    "test-check-message-keys.mjs",
    "test-check-mime-claims-decodable.mjs",
    "test-check-network-scope-marked.mjs",
    "test-check-openable-apps-can-read.mjs",
    "test-check-opener-args.mjs",
    "test-check-optimistic-write.mjs",
    "test-check-peer-identity-sandbox.mjs",
    "test-check-plugin-command-grants.mjs",
    "test-check-plugin-grants.mjs",
    "test-check-portal-interfaces.mjs",
    "test-check-probe-admission.mjs",
    "test-check-profile-agreement.mjs",
    "test-check-profile-case.mjs",
    "test-check-profile-claims.mjs",
    "test-check-profile-keys.mjs",
    "test-check-profile-principals.mjs",
    "test-check-proto-drift.mjs",
    "test-check-read-grants-cover-queries.mjs",
    "test-check-read-scope.mjs",
    "test-check-readme-tree.mjs",
    "test-check-refusal-language.mjs",
    "test-check-refusal-shape.mjs",
    "test-check-release-routes.mjs",
    "test-check-runtime-assets.mjs",
    "test-check-runtime-deps.mjs",
    "test-check-runtime-dir-closed.mjs",
    "test-check-sandbox-env.mjs",
    "test-check-sensing-vectors.mjs",
    "test-check-serde-nesting.mjs",
    "test-check-session-origin.mjs",
    "test-check-setup-runtime.mjs",
    "test-check-shared-env-names.mjs",
    "test-check-shared-files.mjs",
    "test-check-smoke-coverage.mjs",
    "test-check-socket-servers.mjs",
    "test-check-sound-theme.mjs",
    "test-check-spawned-binaries.mjs",
    "test-check-spawned-tools-classified.mjs",
    "test-check-subscribe-scope.mjs",
    "test-check-tests-run.mjs",
    "test-check-toast-is-named.mjs",
    "test-check-token-unions.mjs",
    "test-check-unit-directives.mjs",
    "test-check-unit-identity.mjs",
    "test-check-units.mjs",
    "test-check-unrendered-error.mjs",
    "test-check-untranslated-render.mjs",
    "test-check-user-unit-firewall.mjs",
    "test-check-user-units-started.mjs",
    "test-check-verify-image.mjs",
    "test-check-webview-sandbox.mjs",
    "test-check-window-capability.mjs",
    "test-check-window-grants.mjs",
    "test-check-wired.mjs",
    "test-ci-system-deps.mjs",
    "test-pi-completion-shape.mjs",
    "test-pre-commit-hook.mjs",
    "test-probe-verdict.mjs",
    "test-typecheck-gating.mjs",
}


def main() -> int:
    files = controls()
    if not files:
        print("no control scripts found; the layout moved and this check did not")
        return 1

    problems: list[str] = []
    checked = 0
    present = {n for n in MIGRATED_LATER if (SCRIPTS / n).is_file()}
    # Only in a tree that actually holds these controls. A small fixture holds none
    # of them, and reporting every entry as stale there buries whatever it was
    # testing - the same trap the plugin-grant check fell into.
    stale = sorted(MIGRATED_LATER - present) if present else []

    for path in files:
        raw = path.read_text(encoding="utf-8", errors="replace")
        checked += 1
        # Each rule reads the form its target can appear in. The node call is CODE,
        # so it is matched with strings blanked out; `rm -rf` reaches a shell only
        # ever AS a string, so blanking would make it unfindable.
        found = offenders(without_strings(raw), raw)
        if not found:
            continue
        if path.name in MIGRATED_LATER:
            continue
        problems.append(
            f"{path.relative_to(ROOT)} deletes recursively on its own "
            f"({found[0]}). A control may only remove what it minted: import "
            f"`mint` and `cleanup` from `{HELPER}`, which refuses a path it has no "
            f"record of creating. This is the check written after one of these "
            f"deleted the repository."
        )

    for name in stale:
        problems.append(
            f"{name} is listed in MIGRATED_LATER and is not there any more. "
            f"Drop the entry: a list of files that do not exist hides how much is left."
        )

    if problems:
        for p in problems:
            print(p)
        return 1

    print(
        f"{checked} control(s): every recursive delete goes through {HELPER}, "
        f"{len(MIGRATED_LATER)} still to migrate"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
