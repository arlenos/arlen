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


def wrapped_calls(root: Path, known: set[str]) -> list[str]:
    """Calls this check cannot see, because they go through a local wrapper.

    Both directions here look for the literal `invoke("name", ...)`. A file that
    defines its own helper - `async function send(cmd, args) { return invoke(cmd,
    args) }` - and then calls `send("clock_set_alarm", {alarm})` presents no
    literal to find, so every one of those calls is invisible: its argument shape
    is never compared, and it does not appear in the invoke count either. The
    clock app routes fifteen calls that way, which is most of its backend.

    Rather than resolve them - the wrapper may rename or reshape the payload, so
    following it could be confidently wrong - they are counted and named, so the
    number this check reports comes with the number it could not reach. A gate
    that reports 541 calls checked while 15 more exist is not wrong about the 541;
    it is wrong about what its silence means.
    """
    found: list[str] = []
    for path in (root / "apps").rglob("*"):
        if path.suffix not in (".ts", ".svelte") or not path.is_file():
            continue
        if "node_modules" in path.parts or ".svelte-kit" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        # A helper whose own call to invoke passes a variable, not a literal.
        # Found by walking BACK from each such invoke to the nearest enclosing
        # declaration rather than by matching a function shape forwards: the first
        # attempt matched a brace-balanced body and found three of eighteen,
        # which is worse than finding none - a partial number reads as a total.
        wrappers = set()
        for m in re.finditer(r"\binvoke\s*(?:<[^>]*>)?\s*\(\s*([A-Za-z_$])", text):
            if m.group(1) in ('"', "'"):
                continue
            # EVERY name declared before this invoke is a candidate, not just the
            # nearest: taking only the nearest found the clock's `send` and missed
            # the harness's `run`, whose body declares a local const in between.
            # The second condition below - the name is called somewhere with a
            # literal command - is what keeps the candidate set honest.
            before = text[: m.start()]
            for d in re.finditer(r"(?:function|const|let)\s+(\w+)\s*[=(]", before):
                wrappers.add(d.group(1))
        for w in sorted(wrappers):
            calls = re.findall(rf'\b{re.escape(w)}\s*(?:<[^>]*>)?\s*\(\s*"([a-z_0-9]+)"', text)
            # Only literals that name a command anyone defines. Without this the
            # candidate set reports `hunk("auto")` in the text editor, where the
            # string is a mode and not a command at all - a wrong name in a
            # coverage report is worse than a missing one, because someone will
            # go looking for it.
            calls = [c for c in calls if c in known]
            if calls:
                found.append(
                    f"{path.relative_to(root)}: {len(calls)} call(s) through `{w}()` "
                    f"({', '.join(sorted(set(calls))[:4])}"
                    f"{'...' if len(set(calls)) > 4 else ''})"
                )
    return sorted(found)


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


# Directories that hold code we did not write. `target` was here from the start;
# `mkosi.builddir` was not, and it holds a whole cargo registry - thousands of
# vendored crates. A dependency defining its own `struct Process` or `struct Info`
# collided with ours, the ambiguity rule dropped the name, and the check then
# SKIPPED those types entirely while printing a clean pass. It hid two real bugs
# on every machine that had built the image, and only CI - which has no build
# directory - saw them. A scan that quietly covers less on a developer's machine
# than in CI is worse than one that covers less everywhere, because the developer
# is the one who could still fix it cheaply.
BUILD_DIRS = {"target", "node_modules", "mkosi.builddir", ".git", ".svelte-kit", "build", "dist"}



# Commands the frontend invokes that no `#[tauri::command]` in the tree defines.
# Each one is a call that cannot succeed - but a pile of 64 says nothing about
# which are deliberate and which are rot, and that ambiguity IS the problem. So
# every one is either implemented or declared here with a reason and an owner,
# the same shape `check-shipped-units.py` uses for units the image does not carry.
#
# arlen-ui builds a surface against its intended contract and reports the missing
# command rather than writing the Rust, so "ui built ahead" is a legitimate entry -
# it names work owed, which an undeclared call does not.
#
# The list rots in both directions: implementing a command must delete its entry,
# and an entry for a command nothing invokes any more is equally an error. An
# unattributed entry is not a pass, it is a triage item that has been seen.
DEAD_INVOKES: dict[str, str] = {
    # capture-active #12; the PipeWire producer is host-blocked and builds on the ship image, so the picker and badge have no backend yet - coder, target build
    "list_capture_sources": "capture-active #12; the PipeWire producer is host-blocked and builds on the ship image, so the picker and badge have no backend yet",
    "start_screencast": "capture-active #12; the PipeWire producer is host-blocked and builds on the ship image, so the picker and badge have no backend yet",
    "cancel_screencast": "capture-active #12; the PipeWire producer is host-blocked and builds on the ship image, so the picker and badge have no backend yet",
    "capture_status": "capture-active #12; the PipeWire producer is host-blocked and builds on the ship image, so the picker and badge have no backend yet",
    "stop_capture": "capture-active #12; the PipeWire producer is host-blocked and builds on the ship image, so the picker and badge have no backend yet",
    # the shell's print dialog against the CUPS/IPP backend; the daemon side exists, the shell commands do not - coder
    "poll_print_request": "the shell's print dialog against the CUPS/IPP backend; the daemon side exists, the shell commands do not",
    "submit_print": "the shell's print dialog against the CUPS/IPP backend; the daemon side exists, the shell commands do not",
    "cancel_print": "the shell's print dialog against the CUPS/IPP backend; the daemon side exists, the shell commands do not",
    # The Settings printers page. Measured 8 Aug: the read half is live and the
    # other six have NO backend operation to bridge to - `PrintBackend` is
    # printers/default_printer/jobs/submit/cancel_job and nothing more. Four of
    # them are CUPS admin writes the Settings module doc already defers to an
    # admin extension, so they are a build behind polkit rather than a wire. Each
    # entry says which it is; the old shared reason ("against the same backend")
    # read as a pending bridge and was how the gap stayed comfortable. - coder
    "printers_discover": (
        "the Settings printers page. No backend operation - but unlike its four neighbours this is a READ (DNS-SD network discovery), so it needs no privilege and is separable from the admin extension if the discover button is wanted sooner"
    ),
    "printers_add": (
        "the Settings printers page. NOT a missing bridge: `PrintBackend` has five operations (printers, default_printer, jobs, submit, cancel_job) and no add. Adding a printer is a CUPS admin write needing lpadmin/polkit, which the Settings module doc already defers to a deliberate admin extension"
    ),
    "printers_remove": (
        "the Settings printers page. Same as printers_add: no backend operation exists, and removing a printer is a privileged CUPS admin write, not a wire"
    ),
    "print_job_retry": (
        "the Settings printers page. No backend operation; a retry would compose submit + jobs, which exist, so it is the smallest of the six"
    ),
    # dictation has no backend at all yet; the badge was built against the intended contract - needs a decision on whether dictation is in scope
    "dictation_status": "dictation has no backend at all yet; the badge was built against the intended contract",
    "stop_dictation": "dictation has no backend at all yet; the badge was built against the intended contract",
    # The modules panel was written against a contract modulesd never had. Two of
    # its four calls were only misnamed and now reach `modulesd_set_enabled` and
    # `retry_module`. These two are real gaps: `modulesd_list_modules` exists but
    # returns UiModule (tier/failed/priority/extension_points) while the panel
    # wants description/module_type/source/has_*/icon, and nothing anywhere
    # reports per-module errors. Needs the panel and the daemon to agree a shape
    "list_modules": "modulesd_list_modules exists but returns a different shape than the panel reads",
    "get_module_errors": "no per-module error report exists anywhere in the tree",
    # the Windows-app (bottles) surface; no backend in the tree - needs a decision on whether this ships before the surface is finished
    "windows_file_request": "the Windows-app (bottles) surface; no backend in the tree",
    "windows_file_run": "the Windows-app (bottles) surface; no backend in the tree",
    "windows_file_install": "the Windows-app (bottles) surface; no backend in the tree",
    "list_bottles": "the Windows-app (bottles) surface; no backend in the tree",
    "delete_bottle": "the Windows-app (bottles) surface; no backend in the tree",
    "set_bottle_config": "the Windows-app (bottles) surface; no backend in the tree",
    "browse_bottle_files": "the Windows-app (bottles) surface; no backend in the tree",
    "clear_bottle_caches": "the Windows-app (bottles) surface; no backend in the tree",
    "install_windows_app": "the Windows-app (bottles) surface; no backend in the tree",
    "set_windows_defaults": "the Windows-app (bottles) surface; no backend in the tree",
    # the Settings sentinel page against the anomaly detector; the daemon exists, the settings commands do not - coder
    "sentinel_get_state": "the Settings sentinel page. Measured 8 Aug and the other way round from how this read: the pure detector cores exist (`daemons/sentinel-detect`), the `org.arlen.Sentinel1` daemon they were written for does NOT - the integration harness says so in as many words. There is also no config surface to bridge to: the anomaly detector builds its `DetectorConfig::default()` in `main` and reads no file, so state, sensitivity and per-detector toggles have nothing to read or write. A daemon to build, not a command to register",
    "sentinel_set_alerts": "the Settings sentinel page. Measured 8 Aug and the other way round from how this read: the pure detector cores exist (`daemons/sentinel-detect`), the `org.arlen.Sentinel1` daemon they were written for does NOT - the integration harness says so in as many words. There is also no config surface to bridge to: the anomaly detector builds its `DetectorConfig::default()` in `main` and reads no file, so state, sensitivity and per-detector toggles have nothing to read or write. A daemon to build, not a command to register",
    "sentinel_set_detector": "the Settings sentinel page. Measured 8 Aug and the other way round from how this read: the pure detector cores exist (`daemons/sentinel-detect`), the `org.arlen.Sentinel1` daemon they were written for does NOT - the integration harness says so in as many words. There is also no config surface to bridge to: the anomaly detector builds its `DetectorConfig::default()` in `main` and reads no file, so state, sensitivity and per-detector toggles have nothing to read or write. A daemon to build, not a command to register",
    "sentinel_set_sensitivity": "the Settings sentinel page. Measured 8 Aug and the other way round from how this read: the pure detector cores exist (`daemons/sentinel-detect`), the `org.arlen.Sentinel1` daemon they were written for does NOT - the integration harness says so in as many words. There is also no config surface to bridge to: the anomaly detector builds its `DetectorConfig::default()` in `main` and reads no file, so state, sensitivity and per-detector toggles have nothing to read or write. A daemon to build, not a command to register",
    "sentinel_fix_posture": "the Settings sentinel page. Measured 8 Aug and the other way round from how this read: the pure detector cores exist (`daemons/sentinel-detect`), the `org.arlen.Sentinel1` daemon they were written for does NOT - the integration harness says so in as many words. There is also no config surface to bridge to: the anomaly detector builds its `DetectorConfig::default()` in `main` and reads no file, so state, sensitivity and per-detector toggles have nothing to read or write. A daemon to build, not a command to register",
    # the wallpaper surface against wallpaperd; the daemon exists, these commands do not - coder
    # apps/knowledge has NO src-tauri at all: it is a SvelteKit frontend calling
    # into a host that does not exist, which is why 14 commands are dead rather
    # than 14 commands being unwritten. The daemon-side read ops all exist and the
    # os-sdk has clients for them, so the missing piece is the app's Rust side -
    # or a decision that this app is hosted some other way. Needs a decision
    "knowledge_library": (
        "the Knowledge app. Its Rust side exists; papers, books, notes and mail are bridge-ingested entities that are not graph nodes yet, so there is nothing to read"
    ),
    "knowledge_timeline_pause": (
        "the Knowledge app. Its Rust side exists; pausing recording is a knowledge-daemon capability that does not exist - there is no switch to flip"
    ),
    "knowledge_timeline_delete": (
        "the Knowledge app. Its Rust side exists; deleting recorded history needs a daemon-side retention op and a confirmation path, neither built"
    ),
    "knowledge_capsules": (
        "the Knowledge app. Its Rust side exists; this needs a capsuled client and an expiry the backend states as a timestamp rather than the store's prose ('in 5 days')"
    ),
    "knowledge_capsule_mint": (
        "the Knowledge app. Its Rust side exists; minting is capsuled's human-gated flow, not a read"
    ),
    "knowledge_capsule_revoke": (
        "the Knowledge app. Its Rust side exists; pairs with the mint flow"
    ),
    "knowledge_capsule_preview": (
        "the Knowledge app. Its Rust side exists; pairs with knowledge_capsules and the capsuled client"
    ),
    # the store's update actions against installd - arlen-ui's surface, coder owes the commands
    "store_update": "the store's update actions against installd",
    "store_uninstall": "the store's update actions against installd",
    "store_update_all_routine": "the store's update actions against installd",
    # apps/text-editor is the same shape as apps/knowledge: a frontend with no
    # src-tauri, so these three have nowhere to live yet - needs the same decision
    "ai_edit": "apps/text-editor has no src-tauri; the app has no Rust side to define them in",
    # These three read as unattributed from the call site and are not: each
    # surface's own header names the plan and says it is fixture-backed until the
    # command lands. Read the file, not just the line.
    #
    # job-progress-surface.md: the Activity/Jobs feed. The JobView server (the
    # notification daemon extended into a KDE-JobViewV3 mirror) plus the producers
    # reporting progress are the coder seam; `list_jobs` is its query - coder
    "list_jobs": "job-progress-surface.md; the JobView server and its producers are not built",
    # waypointer-ai-prompt.md: Tab flips the launcher into Ask mode. The call is a
    # read-tier single completion over org.arlen.AIAgent1 - coder
    "waypointer_ask": "waypointer-ai-prompt.md; the read-tier completion call is not built",
    # The capability browser lives in Settings/Privacy (decision 6), so knowledge
    # links out rather than re-hosting it. Needs a cross-app open mechanism, which
    # nothing provides yet - coder, and it is a mechanism rather than one command
}

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


def rust_struct_fields(root: Path) -> tuple[dict[str, set[str]], set[str], dict[str, dict[str, set[str]]]]:
    """Map struct name to its field names, for every struct in the tree.

    Field names only, in snake_case. A struct name defined twice is dropped
    rather than guessed at: comparing against the wrong one is how the argument
    half produced its first round of false findings.

    The dropped names come back with the map, because dropping one is how a
    command silently leaves the return comparison. A pair that cannot be compared
    prints identically to a pair that matched, and that is the difference between
    a check and the appearance of one, so the caller reports them.

    The third return value is the same thing keyed BY APP, and it is what rescues
    most of the dropped names. A Tauri command lives in one app's binary, so a
    call in `apps/files` that returns `Project` means the `Project` under
    `apps/files/` - not the four others in the tree. Comparing within the app is
    both safer and more accurate than the global lookup, which had left 29 pairs
    (a quarter of them) unchecked purely because `SearchResult`, `Session`,
    `Project` and `Capability` are unremarkable names that several apps chose.
    """
    fields: dict[str, set[str]] = {}
    per_app: dict[str, dict[str, set[str]]] = {}
    seen_twice: set[str] = set()
    for path in root.rglob("*.rs"):
        if BUILD_DIRS & set(path.parts):
            continue
        parts = path.relative_to(root).parts
        app = parts[1] if len(parts) > 1 and parts[0] == "apps" else None
        text = path.read_text(encoding="utf-8", errors="replace")
        for m in re.finditer(r"struct\s+(\w+)\s*\{([^}]*)\}", text, re.S):
            name, body = m.group(1), m.group(2)
            got = set()
            renamed: str | None = None
            for line in body.splitlines():
                line = line.strip()
                if not line:
                    continue
                # `#[serde(rename = "memMB")]` is what the field is called ON THE
                # WIRE, which is the only name the frontend ever sees. Skipping
                # attribute lines meant comparing the Rust identifier `mem_mb`
                # against the TS `memMB`, whose snake form is `mem_m_b` - a
                # mismatch reported against two sides that agree exactly.
                rm = re.search(r'serde\s*\(\s*rename\s*=\s*"([^"]+)"', line)
                if rm:
                    renamed = rm.group(1)
                    continue
                if line.startswith(("#", "//", "///")):
                    continue
                # `r#type` is the raw-identifier form of a field whose name is a
                # Rust keyword; serde puts `type` on the wire. Without the `r#`
                # here the struct reads as not producing `type` at all, which is
                # how widening this check to compare within an app produced its
                # first finding - against a pair that agrees exactly.
                fm = re.match(r"(?:pub\s+)?(?:r#)?(\w+)\s*:", line)
                if fm:
                    got.add(snake(renamed or fm.group(1)))
                    renamed = None
            if name in fields and fields[name] != got:
                seen_twice.add(name)
            fields[name] = got
            if app:
                # Within one app a repeat is still ambiguous, so the same
                # drop-rather-than-guess rule applies per app.
                bucket = per_app.setdefault(app, {})
                if name in bucket and bucket[name] != got:
                    bucket[name] = set()  # marked ambiguous within the app
                elif name not in bucket:
                    bucket[name] = got
    for name in seen_twice:
        fields.pop(name, None)
    return fields, seen_twice, per_app


def balanced_body(text: str, open_at: int) -> str:
    """The text between `text[open_at] == '{'` and its matching close.

    A regex stopping at the first `}` reads a nested object as the end of the
    whole block. `Info` declares `conventional: { kind, size, mode, ... }`, and
    reading it that way promoted four of `conventional`'s fields to fields of
    `Info` itself, then reported the Rust side for not producing them. It does
    produce them - one level down, where they belong.
    """
    depth = 0
    for i in range(open_at, len(text)):
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
            if depth == 0:
                return text[open_at + 1 : i]
    return ""


def top_level_fields(body: str) -> set[str]:
    """The field names a body declares at ITS level, skipping nested objects."""
    got: set[str] = set()
    depth = 0
    for raw in body.splitlines():
        line = raw.strip()
        opens, closes = line.count("{"), line.count("}")
        if depth == 0 and line and not line.startswith(("//", "/*", "*", "///")):
            # Only fields the interface declares as REQUIRED. A `field?:` is the
            # frontend saying it already handles absence - the print queue's
            # `progress?` is exactly that, and reporting it would be noise.
            fm = re.match(r"(\w+)\s*:", line)
            if fm:
                got.add(snake(fm.group(1)))
        depth += opens - closes
        if depth < 0:
            depth = 0
    return got


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
        for m in re.finditer(r"interface\s+(\w+)\s*\{", text):
            name = m.group(1)
            got = top_level_fields(balanced_body(text, m.end() - 1))
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


def check_returns(root: Path) -> tuple[int, list[str], list[str], list[str]]:
    """Report interface fields the command's return struct does not produce."""
    returns = rust_return_types(root)
    structs, ambiguous, structs_by_app = rust_struct_fields(root)
    interfaces = ts_interfaces(root)
    problems: list[str] = []
    known: list[str] = []
    uncompared: list[str] = []
    checked = 0
    for app, path, line, tsname, cmd in annotated_calls(root):
        rust_name = returns.get(app, {}).get(cmd)
        if not rust_name or OPAQUE_RETURN.match(rust_name):
            continue
        # The app's own struct first: a command lives in its app's binary, so a
        # name defined there is the one this call means.
        own = structs_by_app.get(app, {}).get(rust_name)
        produced = own if own else structs.get(rust_name)
        declared = interfaces.get(app, {}).get(tsname)
        if produced is None and rust_name in ambiguous:
            uncompared.append(
                f"{path}:{line}: `{cmd}` returns `{rust_name}`, a struct name defined "
                f"more than once outside this app, so its shape is not compared. "
                f"Rename one, or define the returned struct in the app that returns it."
            )
            continue
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
    return checked, problems, known, uncompared


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

    # Every command any app defines, so a call into a command that exists
    # elsewhere reads as a scoping question rather than a dead call.
    defined_anywhere = {c for app_cmds in commands.values() for c in app_cmds}
    invoked_nowhere: dict[str, str] = {}
    for app, path, line, cmd, keys in invoke_calls(root):
        if cmd not in defined_anywhere and cmd not in EXCUSED:
            invoked_nowhere.setdefault(cmd, f"{path}:{line}")

    for cmd, where in sorted(invoked_nowhere.items()):
        if cmd not in DEAD_INVOKES:
            problems.append(
                f"{where}: `{cmd}` is invoked and no #[tauri::command] in the tree "
                f"defines it, so the call cannot succeed. Implement it, or declare it "
                f"in DEAD_INVOKES with a reason and an owner"
            )
    # The table describes THIS repo, so its rot checks only mean anything when
    # scanning it. A fixture tree invokes none of the 64 and would report every
    # entry as stale.
    for cmd in sorted(DEAD_INVOKES if root == ROOT else {}):
        if cmd in defined_anywhere:
            problems.append(
                f"`{cmd}` is declared dead but a command now defines it; delete the entry"
            )
        elif cmd not in invoked_nowhere:
            problems.append(
                f"`{cmd}` is declared dead but nothing invokes it any more; delete the entry"
            )

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

    ret_checked, ret_problems, ret_known, ret_uncompared = check_returns(root)
    problems.extend(ret_problems)

    known = {c for cmds in commands.values() for c in cmds}
    wrapped = wrapped_calls(root, known)
    if wrapped:
        print("calls routed through a local wrapper, which this cannot read:\n")
        for w in wrapped:
            print(f"  - {w}")
        print(
            "    Their argument shape is not compared and they are not in the count\n"
            "    below. Named rather than omitted: a silent gap in a coverage number\n"
            "    reads as coverage.\n"
        )

    print(
        f"{checked} invoke call(s) checked against {total} command(s); "
        f"{ret_checked} annotated return type(s) compared; "
        f"{len(DEAD_INVOKES)} command(s) invoked with no implementation, each declared"
    )
    if problems:
        print("\nshapes that do not match the command on the other side:\n")
        for p in problems:
            print(f"  - {p}")
        return 1
    if ret_uncompared:
        print("\nreturn shapes this could not compare, so nothing about them is known:\n")
        for u in ret_uncompared:
            print(f"  - {u}")
    if ret_known:
        print("\nknown return mismatches, routed to their owners:\n")
        for k in ret_known:
            print(f"  - {k}")
    print("\nevery call passes the arguments its command declares")
    return 0


if __name__ == "__main__":
    sys.exit(main())
