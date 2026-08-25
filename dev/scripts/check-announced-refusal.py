# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that a refusal a person's press caused is announced, not only drawn.

`check-unrendered-error` asks whether a recorded failure has a reader at all.
This asks the next question: when the reader is there, does anyone who is not
looking at the screen learn about it.

Press mute with the audio service refusing and the sound panel says "That change
did not reach the audio service." in a tinted strip along its top edge. A screen
reader says nothing, and the speaker icon has not moved, so the only signal that
the press did not take is one the person cannot see. The strip was a plain `div`;
six panels shared it. Found on 18 August by pressing the button and reading the
accessibility tree instead of the pixels.

WHAT IS IN SCOPE, and why it is drawn this narrowly:

A live region announces a CHANGE. That makes it right for a message that arrives
because of something the person just did, and wrong for a message that is part of
the page when it loads - a read that failed while the page drew is prose a reader
meets in order, and marking it assertive would interrupt for something that was
already there.

So the subject is a LOCAL `$state` flag in a component, named like a failure, that
is assigned somewhere other than its declaration. That is the shape a handler
writes. Flags that arrive through a store (`$somethingUnavailable`) are excluded:
those are the load-time answers, and they are `check-unrendered-error`'s subject.

WHAT COUNTS AS ANNOUNCING:

  * `role="alert"` or `aria-live` inside the block, for a message that appears in
    response to a press.
  * `aria-invalid` / `aria-describedby` naming the flag, for field validation that
    re-runs on every keystroke - an assertive alert there would interrupt on each
    one, so the message is tied to the field instead of shouted.

WHAT THIS DOES NOT COVER, deliberately:

  * Whether the sentence is any good. `role="alert"` on "Error" announces "Error".
  * A refusal that renders no message at all. Nothing to announce is
    `check-optimistic-write`'s and `check-unrendered-error`'s ground.
  * A message that reaches the screen through a toast. Sonner owns that region and
    it is already a live one.
"""

import re
import sys
from pathlib import Path

ROOT = (
    Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
)

#: A local component flag whose name says it holds a failure. `$state` only: a
#: plain `let` is not what a handler flips in this codebase, and a store-backed
#: one is the load-time shape this check deliberately leaves alone.
#: NOT `unavailable`: that word names a LOAD failing, and this check is explicitly
#: about a refusal caused by a press. `apps/settings/.../ai` holds
#: `behavioursUnavailable` in a local `$state` set by `loadBehaviours`, which is
#: `check-unrendered-error`'s subject wearing this one's clothes - a local flag,
#: but not one a handler writes. Including the word would report it, and the fix
#: it asked for (an assertive alert) would interrupt a reader over prose that was
#: on the page before they touched anything.
FAILURE_STATE = re.compile(
    r"let\s+(\w*(?:[eE]rror|[fF]ail|[rR]efus|[dD]enied|[bB]locked"
    r"|[pP]roblem|[iI]nvalid|[uU]nwritable)\w*)\s*=\s*\$state"
)

#: The same flag assigned again, which is what a handler does. The declaration
#: itself is excluded by requiring something other than `$state` on the right.
def assigned_later(source: str, flag: str) -> bool:
    for m in re.finditer(rf"\b{re.escape(flag)}\s*=\s*(.+)", source):
        if not m.group(1).lstrip().startswith("$state"):
            return True
    return False


#: An opening `{#if <flag>}` or a `{:else if <flag>}` on the markup side. Both,
#: because a pane that shows a failure as one of several states writes the second
#: - and matching only the first made mail's refusal invisible to this check for
#: as long as it has existed. It was not reported as silent; it was not read.
BRANCH = re.compile(r"\{#if\s+([^}]*)\}|\{:else\s+if\s+([^}]*)\}")


def if_blocks(markup: str, flag: str) -> list[str]:
    """The TRUTHY branch of every `{#if ...<flag>...}` or `{:else if ...}`.

    Nesting-aware, and it stops at the branch's own `{:else}`: only the branch
    that runs when the failure is real has anything to announce. Reading past it
    is how bluetooth's "Connecting..." fallback made its banner look silent.
    """
    blocks = []
    for m in BRANCH.finditer(markup):
        condition = m.group(1) if m.group(1) is not None else m.group(2)
        if not re.search(rf"\b{re.escape(flag)}\b", condition):
            continue
        depth = 1
        i = m.end()
        cut = None
        while i < len(markup) and depth:
            nxt = re.search(r"\{#if\b|\{/if\}|\{:else", markup[i:])
            if not nxt:
                break
            token = nxt.group(0)
            if token == "{:else":
                if depth == 1 and cut is None:
                    cut = i + nxt.start()
                i += nxt.end()
                continue
            depth += 1 if token.startswith("{#if") else -1
            i += nxt.end()
        blocks.append((m.start(), m.end(), cut if cut is not None else i))

    # A block INSIDE another matched block is not its own claim. `{#if failure}`
    # wrapping `{#if failure.problem === "launch"}` matches twice, because
    # `\bfailure\b` is in both - and the inner branch is only the sentence, while
    # the `role="alert"` is on the paragraph around it. Reported as silent, both
    # the calendar and mail pages looked unannounced while being correct. The
    # OUTER block is the one that owns the announcement, so a nested match is
    # dropped rather than judged on its own.
    outer = [b for b in blocks if not any(o[1] <= b[0] and b[2] <= o[2] for o in blocks if o is not b)]
    return [markup[start:end] for _, start, end in outer]


ANNOUNCED = re.compile(r'role="alert"|aria-live=')

#: Anything rendered at all. A block that draws no text has nothing to announce.
RENDERS = re.compile(r"\{\s*\$?t\(|\{\s*\$?\w+\s*\}|>[^<>{}]*[A-Za-z]{2}[^<>{}]*<")

#: `<SomeComponent ... />` - a block that hands the message to a child instead of
#: drawing it. The role lives in the child, so looking only at the block would
#: report the six panels that share `PopoverErrorBanner` as silent when the fix
#: is already in place one file over.
CHILD = re.compile(r"<([A-Z]\w*)[\s/>]")

#: Both the default import and the braced named one. The kit's shared primitives
#: come through the braced form, so a pattern that only knew the default one
#: could not follow `LiveRegion` to the file that announces.
IMPORT = (
    r'import\s+(?:NAME|\{[^}]*\bNAME\b[^}]*\})\s+from\s+["\']([^"\']+)["\']'
)


def announced_through_child(block: str, head: str, path: Path, root: Path) -> bool:
    """True when ANY component the block renders announces.

    Any, not all: a block that draws the sentence in a `Row` and speaks it
    through a `LiveRegion` beside it is announced, and requiring every child to
    carry the role would call that silent.
    """
    for name in sorted(set(CHILD.findall(block))):
        m = re.search(IMPORT.replace("NAME", re.escape(name)), head)
        if not m:
            continue
        spec = m.group(1)
        if spec.startswith("$lib/"):
            # `src/lib/...`, from the app root this file lives under.
            app_src = path
            while app_src.name != "src" and app_src != root:
                app_src = app_src.parent
            target = app_src / "lib" / spec[len("$lib/") :]
        elif spec.startswith("."):
            target = (path.parent / spec).resolve()
        elif spec.startswith("@arlen/ui-kit/"):
            # The kit is in this tree, and it is where the shared a11y
            # primitives live - `LiveRegion` is the one an app reaches for when
            # the component drawing the message has no attribute to carry it.
            target = root / "sdk/ui-kit/src/lib" / spec[len("@arlen/ui-kit/") :]
        else:
            # A package import: outside this tree, so nothing to read.
            continue
        candidates = (
            sorted(target.rglob("*.svelte"))
            if target.is_dir()
            else [target, target.with_suffix(".svelte")]
        )
        for c in candidates:
            if c.exists() and ANNOUNCED.search(c.read_text(encoding="utf-8", errors="replace")):
                return True
    return False

#: Keys are `app:file:flag`. A deliberate silence belongs here WITH its reason,
#: so the next reader can disagree with the decision rather than rediscover it.
#:
#: Every entry so far is the same shape: a flag named like a failure that only a
#: LOAD path ever sets. The rule above cannot tell those apart from a handler's
#: without following the call graph, so they land here by hand and each one says
#: which function sets it.
ACKNOWLEDGED: dict[str, str] = {
    "desktop-shell:lib/modules/SandboxedModuleHost.svelte:mountError": (
        "Set only inside `onMount`, when the module fails to come up. It is the"
        " first thing in that region when the host draws, so a reader meets it in"
        " order; there is no press for it to be the answer to."
    ),
    "settings:lib/components/displays/BrightnessSection.svelte:loadFailed": (
        "Set only in `reload`, which runs from `onMount` and from a"
        " visibilitychange listener. Coming back to the window is not a press, and"
        " an assertive announcement on every tab switch would be noise."
    ),
    "settings:lib/components/displays/ProfileSection.svelte:loadFailed": (
        "Set in `reload`, which does run after a save or delete - but what fails"
        " there is reading the list BACK, and the write's own refusal is already"
        " announced through `writeFailed` beside it. Announcing both would say the"
        " same event twice."
    ),
    "settings:routes/keyboard/+page.svelte:loadError": (
        "Set only in `reload`, called once from `onMount`. The keyboard page has"
        " no reload button; `lastError`, which a save sets, is the one a press can"
        " cause and it is announced."
    ),
}


def main() -> int:
    root_apps = ROOT / "apps"
    apps = (
        sorted(p for p in root_apps.iterdir() if (p / "src").is_dir())
        if root_apps.is_dir()
        else []
    )
    # The daemons carry frontends too. The file picker is the dialog every app
    # borrows, and it sat outside an `apps/`-only scope for as long as it has
    # existed.
    apps += sorted(
        p
        for p in (ROOT / "daemons").glob("*/*")
        if (p / "src").is_dir() and (p / "package.json").is_file()
    )
    if not apps:
        print(f"NOTHING WAS READ: no frontend under {ROOT / 'apps'} or {ROOT / 'daemons'}", file=sys.stderr)
        return 2

    findings: list[str] = []
    checked = 0
    for app in apps:
        for path in sorted((app / "src").rglob("*.svelte")):
            # `routes/_thing/` is a headless look-mock, reachable only by typing
            # its URL. Nobody meets it with a screen reader.
            if any(p.startswith("_") for p in path.relative_to(app / "src").parts):
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            head, _, markup = text.rpartition("</script>")
            if not head:
                continue
            for m in FAILURE_STATE.finditer(head):
                flag = m.group(1)
                if not assigned_later(head, flag):
                    continue
                blocks = [b for b in if_blocks(markup, flag) if RENDERS.search(b)]
                if not blocks:
                    continue
                checked += 1
                key = f"{app.name}:{path.relative_to(app / 'src')}:{flag}"
                if key in ACKNOWLEDGED:
                    continue
                tied = re.search(
                    rf"aria-invalid=\{{[^}}]*\b{re.escape(flag)}\b|"
                    rf"aria-describedby=\{{[^}}]*\b{re.escape(flag)}\b",
                    markup,
                )
                if tied or all(
                    ANNOUNCED.search(b) or announced_through_child(b, head, path, ROOT)
                    for b in blocks
                ):
                    continue
                findings.append(
                    f"  - {path.relative_to(ROOT)}: `{flag}` is set by a handler and"
                    f" rendered, but nothing announces it. A person who cannot see the"
                    f" message has no signal that the press did not take. Give the"
                    f" message `role=\"alert\"`, or for per-keystroke validation tie it"
                    f" to its field with `aria-invalid` + `aria-describedby`."
                )

    if findings:
        print(
            "A refusal caused by a press is drawn but never announced:", file=sys.stderr
        )
        for f in findings:
            print(f, file=sys.stderr)
        print(
            "\nIf a silence is right, add the key to ACKNOWLEDGED with the reason.",
            file=sys.stderr,
        )
        return 1

    print(
        f"{checked} handler-set refusal flag(s) render a message, and each one is"
        f" announced or tied to its field."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
