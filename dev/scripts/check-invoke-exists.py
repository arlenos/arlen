# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every `invoke("x")` in an app has an `x` its host registers.

Distinct from `check-invoke-shape.py`, which compares the ARGUMENTS of a call
against its command's parameters. That one presumes the command exists. This one
asks whether it does.

A Tauri `invoke` reaches exactly two places: the commands the app's own host
registers, and the commands its plugins register. Nothing else. So an app that
invokes a name neither of them has is not calling a backend that lives elsewhere -
it is throwing, every time, and whatever the catch does is what the user sees.

That is the same defect this week has been about, from the other end. The fixture
sweep found catches that answered a failed read with invented content; this finds
the reads that cannot succeed in the first place. `apps/knowledge` invokes fifteen
commands and its host registers four: minting a capsule, exporting a timeline and
reading a file's provenance are buttons on screen that can only fail.

The inventory below is what was true when this was written, app by app, so a new
one is visible against it rather than lost in a list of fifty. It is not an
excuse-list: every entry is work, and an entry disappears when the command lands.

What this does NOT cover:

  * The reverse direction - a registered command nobody invokes - is reported at
    the end but never fails the check. It used to be unreliable for a reason that
    was only a scan path: a shared helper in `ui-kit` calls a command the app's
    own source never names, and this read `apps/*/src` alone. It now reads
    `sdk/ui-kit/src` too and offers those names to every app as INDIRECT calls,
    which may relax the uncalled list and never enter the strict missing-command
    check. What keeps it advisory is the rest: a name built at runtime, or a call
    from somewhere neither tree covers.
  * An invoke whose name is computed (`invoke(cmd)` where `cmd` is a variable),
    including the local-wrapper shape `check-invoke-shape.py` documents - the clock
    app routes fifteen calls through one helper, which is why its commands look
    uncalled below. That direction only ever HIDES a call, so it can make this
    check miss a missing command; it cannot make it invent one.
  * Whether a command that IS registered works, or whether its arguments match.
    This is about existence, which is the failure that renders as a broken button.
  * `apps/harness` and `apps/store` are arlen-ui's live work. Their names are
    listed with everything else rather than skipped: the shape is the same and
    hiding it would misrepresent the inventory.

Shown to fail before being trusted: add `invoke("nonexistent_command")` to any app
under `apps/*/src` and it names that app and that command.
"""

import os
import re
import sys
from pathlib import Path

# The tree to scan. An argument points it at a throwaway one, which is what lets
# `test-check-invokes.mjs` watch the gate fail on a call with no handler and stay
# quiet on one that has it. A gate nobody has seen fail is a gate nobody should
# trust, and this one decides whether a control is honest.
ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

# The generic is optional and may itself be generic: `invoke<ReadOutcome<Row>>(...)`
# is a real shape in this tree, and `<[^>]*>` stops at the first `>` and matches
# nothing - so those calls went invisible in BOTH directions the day one was
# written. One level of nesting is enough for anything here; deeper would want a
# parser, and a parser for this is not worth it.
INVOKE = re.compile(
    r'invoke(?:<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\(\s*["\'`]([A-Za-z_][A-Za-z0-9_]*)["\'`]'
)

# `invoke(command, {...})` - the name arrives as a variable, so the literal is
# wherever that variable was assigned. Settings' module store does exactly this:
#
#     const command = source === "builtin" ? "waypointer_set_plugin_enabled"
#                                          : "modules_set_enabled";
#     await invoke(command, { id, enabled });
#
# Both commands are live and registered, and the literal scan above sees neither.
# That is declared for the missing direction ("string literals only") and was NOT
# declared for the uncalled one, where it does real damage: a command in daily use
# is reported as called by nothing, and the obvious next move on such a report is
# to delete it. I made the same mistake by hand on `modules_set_enabled` the day
# before writing this, reading a route instead of the store it imports.
INVOKE_VAR = re.compile(
    r"invoke(?:<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\(\s*([A-Za-z_][A-Za-z0-9_]*)\s*[,)]"
)
STRINGS = re.compile(r"""["'`]([A-Za-z_][A-Za-z0-9_]*)["'`]""")


def without_template_literals(text: str) -> str:
    """The source with backtick-delimited spans blanked out.

    A template literal can hold a DOCUMENT rather than code, and the text editor
    ships two: its demo files are backtick-delimited constants holding example
    Arlen code, `invoke("Authorize", …)` among it. That is sample content shown
    to a reader, not a call this binary makes - and it was carried on the
    missing-command list for weeks as "the AI-edit gate call", a piece of work
    nobody could ever finish because there was nothing to wire.

    Blanking the span rather than deleting it keeps every other offset intact, so
    a real `invoke` on the same line is still found. Escapes are honoured; a
    `${...}` interpolation is left blanked too, which loses a real call written
    inside one - unlikely, and the safe direction is to under-report a call site
    rather than to invent one.
    """
    out = []
    i = 0
    in_tick = False
    while i < len(text):
        c = text[i]
        if in_tick:
            if c == "\\":
                out.append(" ")
                if i + 1 < len(text):
                    out.append(" ")
                i += 2
                continue
            if c == "`":
                in_tick = False
                out.append(c)
            else:
                out.append("\n" if c == "\n" else " ")
            i += 1
            continue
        if c == "`":
            in_tick = True
        out.append(c)
        i += 1
    return "".join(out)


def indirect_calls(text: str) -> tuple[set[str], set[str]]:
    """Names that reach `invoke` other than as a literal at the call.

    Returns (assigned, wrapped), and the split is the point. Both relax the
    uncalled list; only `wrapped` may also enter the strict missing-command
    check, because only `wrapped` carries a proof.

    `assigned` is literals held in a variable that is later passed to `invoke`.
    That variable's initialiser holds strings which are not command names
    (`"builtin"` above is a discriminant), so treating them as invoked would
    manufacture failures for commands nobody ever calls. One-way, always: the
    cost of a name wrongly kept is nothing, the cost of one wrongly dropped is a
    deleted command.

    `wrapped` is different in kind. A literal at a call site of a helper whose
    body passes its own first parameter to `invoke` reaches `invoke` as its
    first argument by construction - it is a command name or the app throws.
    """
    assigned: set[str] = set()
    for var in set(INVOKE_VAR.findall(text)):
        for m in re.finditer(rf"\b(?:const|let|var)\s+{re.escape(var)}\s*=([^;\n]*(?:\n[^;]*)?);", text):
            assigned |= set(STRINGS.findall(m.group(1)))

    # One hop through a wrapper, the same shape `check-dbus-method-names.py`
    # follows. An app that routes every command through one helper -
    #
    #     async function send(cmd: string, args?: unknown) { await invoke(cmd, args); }
    #     await send("clock_set_alarm", { alarm });
    #
    # showed EVERY one of its commands as uncalled: the literal is at the
    # helper's call sites, and only the parameter is at `invoke`. The clock alone
    # put a dozen live commands on that list, which is how a list stops being
    # read.
    #
    # `finditer`, not `findall`: the pattern needs a second capturing group for
    # the backreference that ties the parameter to `invoke`'s argument, so
    # `findall` would yield tuples.
    wrapped: set[str] = set()
    for wm in WRAPPER.finditer(text):
        wrapper, params = wm.group(1), wm.group(2)
        body = braced(text, text.find("{", wm.end() - 1))
        # The body is taken by BRACE MATCHING, not by a pattern that stops at the
        # first `}`. The editor's hunk driver opens a callback before it reaches
        # `invoke`, so a stop-at-the-first-brace scan saw no forwarding at all and
        # three unregistered commands read as uncalled. Two different reasons for
        # the same green, in one function.
        fm = re.search(r"\binvoke\s*(?:<[^<>]*(?:<[^<>]*>[^<>]*)*>)?\(\s*(\w+)\b", body)
        if not fm:
            continue
        names = [q.strip().split(":")[0].strip() for q in split_args(params)]
        if fm.group(1) not in names:
            continue
        # WHICH parameter, not "the first". `driveHunk(index, next, cmd, failure)`
        # takes the command third, so a first-argument rule read its call sites as
        # no call at all.
        at = names.index(fm.group(1))
        for m in re.finditer(rf"\b{re.escape(wrapper)}\s*\(", text):
            args = split_args(braced(text, m.end() - 1))
            if len(args) <= at:
                continue
            arg = args[at].strip()
            if len(arg) >= 2 and arg[0] == arg[-1] and arg[0] in "\"'":
                wrapped.add(arg[1:-1])
    return assigned, wrapped


def braced(text: str, open_at: int) -> str:
    """The text inside a balanced bracket pair starting at `open_at`.

    Empty when the bracket is missing or never closes, so a truncated or
    unparsable file yields nothing rather than a wrong span - which for the
    strict direction would harvest the wrong literal and manufacture a finding.
    """
    if open_at < 0 or open_at >= len(text) or text[open_at] not in "([{":
        return ""
    depth, i = 0, open_at
    while i < len(text):
        c = text[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                return text[open_at + 1 : i]
        i += 1
    return ""


def split_args(text: str) -> list[str]:
    """Split an argument list on TOP-LEVEL commas.

    Naive `split(",")` would cut inside `{ a: 1, b: 2 }` and inside a nested
    call, and the position this feeds is only right if the count is. Quotes are
    tracked too, so a comma inside a string does not shift every argument after
    it by one - which would harvest the wrong literal and manufacture a finding.
    """
    out, depth, quote, cur = [], 0, "", []
    i = 0
    while i < len(text):
        c = text[i]
        if quote:
            if c == "\\":
                cur.append(c)
                i += 1
                if i < len(text):
                    cur.append(text[i])
                    i += 1
                continue
            if c == quote:
                quote = ""
        elif c in "\"'`":
            quote = c
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            out.append("".join(cur))
            cur = []
            i += 1
            continue
        cur.append(c)
        i += 1
    out.append("".join(cur))
    return out
# A function whose body passes one of its own parameters to `invoke`: the helper
# every command in that file goes through. Captures the helper's name, its whole
# parameter list, and the parameter that reaches `invoke` - the caller then reads
# the literal at THAT position rather than assuming the first.
# Only the `function` form. An arrow helper (`const send = async (cmd) => ...`)
# is not matched, and that is a stated limit rather than a silent one: it would
# read as uncalled, which is the direction that costs nothing.
WRAPPER = re.compile(r"(?:async\s+)?function\s+(\w+)\s*\(([^)]*)\)", re.S)

# Helpers that invoke on their caller's behalf and live in ANOTHER file.
#
# Both wrapper finders above walk one file: they see `invoke(someVar)` and look
# BACKWARDS through the same text for the declaration. A helper that is imported
# is never declared in the file that calls it, so it is invisible to them, and
# every command routed through it reads as invoked by nobody.
#
# That is not hypothetical. `shellAction` was extracted so a control could say
# when its command was refused, and the moment a call moved onto it this check
# reported the command as uncalled and told me to delete the entry that carries
# it. A gate that says "delete this" about live code is worse than no gate.
#
# The value is the argument index holding the command name. One entry today; the
# shape is here so the next helper is one line rather than a rediscovery.
IMPORTED_INVOKERS: dict[str, int] = {
    "shellAction": 0,
}

# The same literal-first-argument shape as `INVOKE`, for those helpers.
IMPORTED_INVOKE = re.compile(
    r"\b(?:%s)\s*\(\s*[\"'`]([A-Za-z_][A-Za-z0-9_]*)[\"'`]"
    % "|".join(re.escape(n) for n in IMPORTED_INVOKERS)
)

HANDLER = re.compile(r"generate_handler!\s*\[(.*?)\]", re.S)

# The commands with no host, as of 9 August, with what each one is. Keeping the
# reason next to the name is the difference between an inventory and an alibi.
#
# EVERY REASON SHOULD CARRY ITS OWN FALSIFIER, written as `FALSE WHEN:`. The
# convention was added on 4 September after five entries in two days turned out
# to describe a state that had ended - a dialog the portal "does not consult"
# that Tim had decided to build, a job contract that was already written, a
# thumbnailer that existed, a confinement belonging to a different process. Each
# read as a verdict because it was written as a description, and a description
# ages while a name does not.
#
# A falsifier is one sentence naming what somebody could go and check. It costs a
# line and it turns "this is blocked" into "this is blocked, and here is how you
# would know it is not any more", which is the difference between a reader
# believing the list and a reader testing it.
KNOWN: dict[str, dict[str, str]] = {
    # The mailbox MODEL, which `mail-app.md` §6 leaves open: what a folder is on
    # this machine and what an id names. Not a missing function - a shape nobody
    # has decided, and writing one into a command would decide it by accident.
    # The store answers a real host with an empty, honestly unconnected mailbox
    # rather than a sample, so the catch does not invent a mailbox either.
    "mail": {
        # FALSE WHEN: `mail-app.md` §6 names a mailbox model. It currently says in
        # its own words that the section "produced no surviving claims, so any
        # scope list I wrote now would be convention dressed as research", and §7
        # adds that encryption is the last blocker to calling the strand planned.
        "mail_folders": (
            "the mailbox model is undecided (mail-app.md §6); a real host gets an "
            "unconnected mailbox rather than a sample"
        ),
        "mail_list": (
            "the mailbox model is undecided (mail-app.md §6); a real host gets an "
            "unconnected mailbox rather than a sample"
        ),
        "mail_open": (
            "the mailbox model is undecided (mail-app.md §6); a real host gets an "
            "unconnected mailbox rather than a sample, and a failed open says so"
        ),
        "mail_sender_person": (
            "NEEDS A PRODUCER: `shared.Person` has an owner and a decided contract "
            "(contacts-decision.md - mail reads people, never owns them) and "
            "nothing writes one yet, so the command would answer nobody to every "
            "address until contacts or the CardDAV bridge exists"
        ),
    },
    # The file picker's two entries are gone as of 4 September, and what they said
    # is worth keeping because both reasons were wrong in the same way. They read
    # "NEEDS A PRODUCER: the portal daemon has no recent-files source and no
    # thumbnailer, and the picker is confined to the daemon's cap-std root". The
    # confinement is the DAEMON's; the picker-ui lists directories with plain
    # `tokio::fs` over absolute paths. The thumbnailer existed in `apps/files/core`
    # with a sandboxed worker behind it, and Recent needed no producer at all once
    # it was read as what a sidebar place is - a folder, remembered by the picker
    # itself, rather than the system's recent files.
    #
    # The lesson for the entries below: a reason that names another component's
    # constraint should say how it was checked, because the next reader cannot
    # tell a measurement from an assumption.
    "knowledge": {
        # FALSE WHEN: a bridge runs on a booted image and writes entity rows - the
        # Obsidian one exists as an EXAMPLE with no image step and no unit, so
        # today the read would return four empty sections on every machine.
        # `knowledge-app.md` decision 8 sequences it exactly this way.
        "knowledge_library": (
            "the library view. Traced: the bridge-ingest daemon writes into dynamic "
            "entity tables under a namespace, and the schema registry lists the "
            "types, so the read is straightforward. What is not is the mapping from "
            "namespace to the store's four sections and which field is the display "
            "title - both are schema decisions that every future bridge inherits"
        ),
    },
    "settings": {
        # ONE Windows-app entry is left of the seven that stood here, and the note
        # that explained them has to be read with its date. It said the page came
        # off the navigation on 9 August because "its backend is the Wine bottle
        # daemon, which `wine-proton-plan.md` defers on purpose", and that nobody
        # could press these controls because there was no way in.
        #
        # Both halves of that have since changed. `daemons/bottled` exists and
        # answers: it makes bottles, installs into them, picks a program, launches
        # it, cuts a bottle off the network, takes a folder away and measures the
        # disk a prefix holds. And the second reason the note gave - the page
        # arguing with itself, a default Wine version sitting two rows above
        # "Runtimes not known" - is gone; the compat section is behind the recipe
        # that would fill it and says so when there is none.
        #
        # What has NOT changed is the way in: `navigation.ts` still has no
        # windows-apps entry, deliberately, with a comment saying it "comes back
        # with the daemon". So the whole panel is reachable only by typing the
        # route. That is now a navigation decision rather than a missing backend,
        # and it belongs to whoever owns the panel's surface.
        # Half of what this said stopped being true on 4 September: the panel has
        # a navigation row again, so it is no longer reachable only by typing the
        # route. What remains is the whole of the reason.
        "set_bottle_config": (
            "bottle settings. Needs the compat recipe, which is its own piece "
            "(forage-distributed and signed) and does not exist - so there is no "
            "measured value for these controls to write. A switch drawn from an "
            "invented default writes to a bottle that does not hold it. FALSE WHEN: "
            "a recipe format exists and a bottle records a Wine version, DLL "
            "overrides or a window mode that can be read back"
        ),
        # The four printer entries that stood here are gone rather than fixed, and
        # the way they were wrong is worth keeping: the CUPS backend exists and
        # `printers_list`/`printers_default` reach it, so "this is just wiring" was
        # true of the half that already worked - which is the most convincing form
        # a wrong claim takes. `PrintBackend` had five operations and none of the
        # four was among them. `print_job_retry` became the sixth (IPP Restart-Job:
        # your own job, no new privilege). The other three came off the Settings
        # panel instead: add and remove are queue administration wanting lpadmin,
        # which is a privilege decision to take on its own terms, and discover is a
        # DNS-SD listener, which is a subsystem rather than a command.
    },
    "desktop-shell": {
        # The whole capture group waits on ONE missing piece rather than five.
        # The ScreenCast portal backend exists (sessions, source-type and cursor
        # negotiation, a content-free audit of every share step); its own header
        # says what is left is "the PipeWire producer that makes `Start` return
        # real node ids". Until that producer exists there is nothing for these
        # shell commands to hand back, so they are one job, not five.
        # FALSE WHEN: a PipeWire producer exists and the portal's `Start` returns
        # real node ids - at which point `arlen.portal` can list ScreenCast in its
        # `Interfaces` line, which is the single check for whether this is still
        # true. Re-measured 4 Sep: no `pipewire` dependency anywhere in the tree.
        "cancel_screencast": "cancelling a screencast (PipeWire producer)",
        "start_screencast": "starting a screencast (PipeWire producer)",
        "stop_capture": "stopping a capture (PipeWire producer)",
        "capture_status": "the capture indicator (PipeWire producer)",
        "list_capture_sources": (
            "the capture source picker - also needs the compositor to enumerate "
            "monitors and toplevels for it"
        ),
        # Dictation needs a speech engine, which this system does not have and has
        # not decided to have. Listed so it is not mistaken for plumbing.
        # FALSE WHEN: a speech engine is provisioned on the image - a binary or
        # model this tree ships and starts, not a crate somebody could add.
        "dictation_status": "the dictation indicator (no speech engine)",
        "stop_dictation": "stopping dictation (no speech engine)",
    },
    "store": {
        # The store app is arlen-ui's live surface, so these three are theirs to
        # wire rather than unowned work sitting on a list. Saying whose they are is
        # the difference between a queue and a census: without it they read as
        # three items nobody has picked up.
    },
    "text-editor": {
        # FALSE WHEN: the gate registry classifies an edit action AND `executor_live`
        # is on. Measured 4 Sep and worth writing down rather than re-deriving:
        # the store models hunks the assistant ALREADY APPLIED on its own, which is
        # the autonomous half and is exactly what that flag gates. A propose-only
        # version - every hunk held for confirm - needs neither, and is a different
        # feature from the one the surface draws rather than a smaller cut of it.
        "ai_edit": (
            "proposing an assistant edit (the gated edit path, executor-live)"
        ),
        # The three below are the same unbuilt path's accept / reject / undo, and
        # they became visible only when the wrapper hop learned to follow a
        # command that arrives at a helper's THIRD parameter. They are unreachable
        # rather than broken: `proposeEdit` catches the missing `ai_edit`, sets
        # `unavailable`, and the review renders no hunks - so there is no button to
        # press. They land with `ai_edit`, and if that one is ever registered
        # without them the review gets an Accept that throws, which is why they are
        # written down separately instead of folded into its line.
        "ai_edit_accept": "applying one reviewed hunk; unreachable until `ai_edit` exists",
        "ai_edit_reject": "holding one reviewed hunk back; unreachable until `ai_edit` exists",
        "ai_edit_undo": "compensating an applied hunk; unreachable until `ai_edit` exists",
    },
}


def handler_names(text: str) -> set[str]:
    """The command names a file registers, with module paths and comments removed."""
    out: set[str] = set()
    for m in HANDLER.finditer(text):
        body = "\n".join(line.split("//")[0] for line in m.group(1).splitlines())
        for part in body.replace("\n", " ").split(","):
            part = part.strip()
            if part:
                out.add(part.split("::")[-1])
    return out


def main() -> int:
    plugin: set[str] = set()
    for f in (ROOT / "sdk" / "tauri-plugin-shell").rglob("*.rs"):
        plugin |= handler_names(f.read_text(encoding="utf-8", errors="replace"))

    findings: list[str] = []
    inventory = 0
    uncalled: list[str] = []
    # `apps/` plus the frontends under `daemons/`: the file picker is a Tauri
    # frontend with its own `src-tauri`, so the pairing this check does - what the
    # UI invokes against what the host registers - applies to it unchanged. It was
    # outside the scope for as long as it existed, and the first run that included
    # it found a command nothing registers.
    apps = sorted(p for p in (ROOT / "apps").iterdir() if (p / "package.json").exists())
    apps += sorted(
        p
        for p in (ROOT / "daemons").glob("*/*")
        if (p / "package.json").exists() and (p / "src-tauri").is_dir()
    )

    # Commands the SHARED KIT invokes, read once and offered to every app.
    #
    # `apps/*/src` alone was the reason the reverse direction carried a
    # "helper-fed" bucket a person had to keep by hand: a ui-kit component calls
    # the command, the app that mounts it never names it, and the scanner then
    # reported a registered command nobody invokes. Two names live there today,
    # `get_surface_tokens` and `pick_directory`.
    #
    # They join `indirect`, never `calls`, and that distinction is the whole
    # safety of this: `indirect` may only RELAX the uncalled list. Put in `calls`
    # they would enter the strict missing-command check, and an app that mounts no
    # kit component using them would be told it is missing a command it never asked
    # for - a gate inventing work, which is worse than the bucket it replaces.
    kit_calls: set[str] = set()
    kit_src = ROOT / "sdk/ui-kit/src"
    if kit_src.exists():
        for f in list(kit_src.rglob("*.ts")) + list(kit_src.rglob("*.svelte")):
            text = without_template_literals(
                f.read_text(encoding="utf-8", errors="replace")
            )
            kit_calls |= set(INVOKE.findall(text))
            kit_calls |= set(IMPORTED_INVOKE.findall(text))


    for app in apps:
        calls: set[str] = set()
        # Names reaching `invoke` through a variable. Kept apart from `calls` so
        # they can relax the uncalled list without ever entering the strict
        # missing-command check.
        indirect: set[str] = set(kit_calls)
        src = app / "src"
        if src.exists():
            for f in list(src.rglob("*.ts")) + list(src.rglob("*.svelte")):
                text = without_template_literals(
                    f.read_text(encoding="utf-8", errors="replace")
                )
                calls |= set(INVOKE.findall(text))
                calls |= set(IMPORTED_INVOKE.findall(text))
                assigned, wrapped = indirect_calls(text)
                indirect |= assigned
                # A wrapper call is a real call, so it is held to the real rule.
                # This was one-way for its first day, inheriting the caution the
                # ASSIGNED case genuinely needs, and the caution does not transfer:
                # a literal at `send("...")` reaches `invoke` as its first argument
                # or the app throws. Left relaxing-only, a typo inside a wrapper
                # call - the exact shape this scanner cannot see at the call site -
                # would throw for every user and keep the gate green, which is the
                # failure this check exists to prevent.
                calls |= wrapped
        handlers: set[str] = set()
        host = app / "src-tauri"
        if host.exists():
            for f in host.rglob("*.rs"):
                handlers |= handler_names(f.read_text(encoding="utf-8", errors="replace"))

        known = KNOWN.get(app.name, {})
        for name in sorted(calls - handlers - plugin):
            if name in known:
                inventory += 1
                continue
            findings.append(
                f"{app.relative_to(ROOT)}: invokes `{name}`, which neither its host nor a "
                f"plugin registers. Every call throws; whatever the catch does is "
                f"what the user gets."
            )
        # An inventory entry whose call is gone. The entry says "this control is on
        # screen with nothing behind it", and once the call goes the sentence is
        # false - it then reads as remaining work that nobody owes. Same shape as a
        # skip-list entry outliving its subject, which is a lie that accumulates.
        for name in sorted(set(known) - calls):
            findings.append(
                f"apps/{app.name}: `{name}` is carried as known-missing and nothing "
                f"invokes it any more. Drop the entry; the count is supposed to be "
                f"what is actually claimed on screen."
            )
        # The other direction, and the one that bit: an entry whose command now
        # EXISTS. The loop above simply stops counting it, so the total quietly
        # drops by one and the entry sits there forever claiming a control is dead
        # that someone has since wired up. An exception must not outlive its
        # subject either way round, and the inventory is only worth reading if it
        # shrinks deliberately.
        for name in sorted(set(known) & (handlers | plugin)):
            findings.append(
                f"apps/{app.name}: `{name}` is carried as known-missing and its "
                f"host now registers it. Drop the entry; a fixed command left in "
                f"the inventory makes the count read higher than the real debt."
            )
        for name in sorted(handlers - calls - indirect):
            uncalled.append(f"apps/{app.name}: `{name}`")

    print(
        f"{len(apps)} app(s) checked that every invoked command exists. "
        f"{inventory} known-missing command(s) carried as inventory; string "
        f"literals only, so a computed command name is invisible here."
    )
    if findings:
        print("\ninvokes with no command behind them:\n")
        for f in findings:
            print(f"  - {f}")
        return 1
    # Printed on request rather than always: 111 lines of informational output
    # every run trains a reader to skip the whole block, including the strict
    # findings above it. `ARLEN_LIST_UNCALLED=1` when somebody is actually
    # pruning.
    if uncalled and os.environ.get("ARLEN_LIST_UNCALLED"):
        print("\nregistered, with no invoke this scanner can see:\n")
        for u in uncalled:
            print(f"  - {u}")
    print(
        f"\n{len(uncalled)} registered command(s) nothing under apps/*/src invokes.\n"
        f"    Informational, and worth knowing WHY before chasing one. Four different\n"
        f"    things land on this list and only one of them is a defect:\n"
        f"      dead        nothing reaches it at all. `waypointer_search`, superseded\n"
        f"                  by the plugin-scoped `waypointer_search_plugin`.\n"
        f"      dispatched  the host calls it itself, keyed by an id the frontend\n"
        f"                  sends elsewhere. `toggle_caffeine` runs on every click of\n"
        f"                  the shell's badge, through `quick_action_run`.\n"
        f"      helper-fed  a ui-kit helper this scanner cannot see calls it, or the\n"
        f"                  name is built at runtime from something other than a\n"
        f"                  plain assignment.\n"
        f"      ahead       a built backend whose surface does not exist yet, which\n"
        f"                  claims nothing on screen and lies to nobody.\n"
        f"    Three sweeps rediscovered that split one name at a time before it was\n"
        f"    written down. Read the caller before deleting anything."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
