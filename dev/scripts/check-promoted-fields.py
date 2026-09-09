#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""Check that a payload field the graph never reads is one somebody meant to drop.

WHAT THIS IS FOR. An app puts a field on the bus and the knowledge daemon decides
what becomes a graph node. Whatever the handler does not read is simply gone -
silently, with no error anywhere, and the app cannot tell. That is the widest
kind of quiet loss in the tree: the sender is correct, the receiver is correct in
isolation, and the fact is missing.

It cost a real defect on 9 September. `promote_timeline_record` wrote the
payload's LABEL into the node's subject and never read `subject` at all, so the
file path an app recorded never reached the graph and "what did I save" had no
answer. Two docs in `os-sdk/src/timeline.rs` disagreed about which field it
should be and the code followed the wrong one. Nothing failed; an integration
scenario querying for the path is what found it.

The same sweep found a second: `PresenceParams.project` promised that an empty
value "inherits Focus Mode's active project (resolved on the daemon side)", and
the daemon does not consume the focus events at all.

WHAT IT CHECKS. For every `promote_*` handler in the knowledge daemon: decode the
payload type it names, take that message's fields out of the proto, and require
each field the handler body never mentions to be listed below with a reason. A
new proto field nobody promotes then fails until somebody says whether it should.

WHAT IT DOES NOT CHECK. Whether the field is read into the RIGHT column - the
timeline bug was a handler reading a field and writing it to the wrong place,
which this would not have caught on its own. What it does is make the second half
visible: `subject` sat in the unread list where a reader would have asked why.

Run: dev/scripts/check-promoted-fields.py [root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]

PROMOTION = "daemons/knowledge/src/promotion.rs"
#: THE DAEMON'S OWN COPY, not the SDK's. They are kept in step deliberately (see
#: `check-proto-drift`), but the handlers below decode the types generated from
#: THIS file, and three payloads exist only here - so reading the SDK's copy
#: reported them as undefined rather than as unread.
PROTO = "daemons/knowledge/proto/event.proto"

#: `handler.field` to why the graph does not carry it. A field NOT listed here and
#: not read by its handler fails.
CARRIED: dict[str, str] = {
    # The kernel's own detail, deliberately not modelled as graph facts.
    "promote_file_opened.flags": "the open mode; promotion keys on the path and a File node models the file rather than one open of it",
    "promote_process_started.pid": "this handler works at the CGROUP level on purpose - it resolves both ends to Apps and drops a self-edge, and its comment says why a pid would only add noise",
    "promote_process_started.ppid": "same, the parent is resolved through its cgroup",
    "promote_process_started.comm": "same; the binary name is not the app identity the graph uses",
    "promote_process_started.exit_code": "this branch only promotes `started`",
    "promote_window_focused.prev_app_id": "which app lost focus is context for the event log; the node is about the app that gained it",
    "promote_service_transition.detail": "free-form text from the unit; the node carries the transition, not the message",
    # Held in the SQLite event row rather than on the node, which is the
    # documented split: the graph stays lightweight and the row keeps the rest.
    "promote_presence_set.metadata": "documented to stay in the event log; the node carries activity + subject",
    "promote_presence_set.auto_clear": "a hint to whoever clears the presence, not a fact about it",
    "promote_presence_set.project": "not resolved and not stored - the daemon consumes no focus events and UserAction has no project column. The field's doc claimed the opposite until 9 September and now says this",
    "promote_timeline_record.metadata": "documented to stay in the event log, like presence's",
    "promote_timeline_record.label": "the human summary; the node carries the TYPE as its action and the SUBJECT as its subject, which is the correction made on 9 September",
    "promote_annotation_set.app_id": "the namespace is the app's own identity here, and the node is keyed by target + namespace",
    "promote_annotation_cleared.app_id": "same",
    # Badges: only error and warning are recorded at all (badges-api.md FA3), so
    # the count-shaped fields have nothing to land in.
    "promote_badge_set.variant": "the handler filters on `status`; a count-only badge is deliberately not promoted",
    "promote_badge_set.count": "count-only badges are deliberately not recorded (badges-api.md FA3)",
    "promote_badge_set.progress_value": "same, a progress badge is not a graph fact",
    "promote_action_invoked.window_id": "which of an app's windows was clicked; the action belongs to the app",
    # From the daemon's own proto, which carries three payloads the SDK's copy
    # does not.
    "promote_process_started.exe_path": "the same cgroup-level reasoning as `comm`: the binary path is not the app identity the graph keys on",
    "promote_process_started.parent_exe_path": "same, for the parent",
    "promote_file_written.bytes": "how much was written; a File node models the file, and a size that changes with every write is a measurement rather than a fact about it",
    "promote_focus_left.prev_app_id": "which app lost focus; this row records that focus left and what kind of thing took it, which is the one thing the event carries that the timestamp does not already imply",
}


def proto_messages(text: str) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    for m in re.finditer(r"message (\w+) \{(.*?)\n\}", text, re.S):
        out[m.group(1)] = re.findall(
            r"^\s*(?:optional\s+)?(?:repeated\s+)?[\w.<>, ]+?\s+(\w+)\s*=\s*\d+;",
            m.group(2),
            re.M,
        )
    return out


def main() -> int:
    src_path, proto_path = ROOT / PROMOTION, ROOT / PROTO
    if not src_path.is_file() or not proto_path.is_file():
        print(f"check-promoted-fields: no promotion pass under {ROOT}", file=sys.stderr)
        return 2

    src = src_path.read_text(encoding="utf-8")
    messages = proto_messages(proto_path.read_text(encoding="utf-8"))

    findings: list[str] = []
    seen: set[str] = set()
    handlers = 0
    for h in re.finditer(r"async fn (promote_\w+)\((.*?)\n\}\n", src, re.S):
        name, body = h.group(1), h.group(2)
        decoded = re.search(r"let (\w+) = (\w+Payload)::decode", body)
        if not decoded:
            continue
        var, message = decoded.group(1), decoded.group(2)
        fields = messages.get(message)
        if fields is None:
            findings.append(f"  - {name} decodes `{message}`, which this proto does not define")
            continue
        handlers += 1
        for field in fields:
            # `r#type` in Rust for the proto's `type`.
            if re.search(rf"\b{re.escape(var)}\.(?:r#)?{re.escape(field)}\b", body):
                continue
            key = f"{name}.{field}"
            seen.add(key)
            if key not in CARRIED:
                findings.append(
                    f"  - {key}: the app sends it and the graph never reads it. Promote it, "
                    f"or list it in this check with what happens to it instead."
                )

    stale = sorted(k for k in CARRIED if k not in seen)
    if stale:
        print("This check explains a field that is read now, or a handler that is gone:\n")
        for k in stale:
            print(f"  - {k}")
        print("\nTake the entry out; an explanation for something that no longer happens is noise.")
        return 1

    if findings:
        print("A payload field reaches the daemon and never reaches the graph:\n")
        print("\n".join(findings))
        print(
            "\nNothing errors when this happens: the app is correct, the handler is "
            "correct on its own, and the fact is simply missing."
        )
        return 1

    if handlers == 0:
        print("check-promoted-fields: no promotion handler decoded a payload", file=sys.stderr)
        return 2

    print(
        f"{handlers} promotion handler(s); every payload field is promoted or "
        f"explained ({len(CARRIED)} explained)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
