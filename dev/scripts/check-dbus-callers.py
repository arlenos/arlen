# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Check that every D-Bus method can see who called it, or says why it need not.

A `#[zbus::interface]` method learns its caller through `#[zbus(header)]`: the bus
attests the sender, and `GetConnectionUnixProcessID` turns that into a pid the
caller cannot forge. A method that does not take the header cannot know who is
asking, whatever it does with the answer.

That is not automatically wrong. A read-only property returning the battery
percentage does not care, and an interface the system calls INTO (a BlueZ agent)
has no app caller at all. It is wrong when the method returns something scoped to
the user, because the session bus is default-allow and `arlen-run` binds its
socket into every confined app: `org.arlen.AI1.explain_system` took no header and
answered with the active app, the current project and recently opened files, and
`completed_actions` returned entries whose `from`/`to` are `type/id` with a File
node's id being its path. Both were reachable by any confined app, and both were
found by running this check by hand rather than by anything failing.

So the rule is not "always take the header". It is that a header-less method has
to be listed below with the reason it is safe, which makes adding one a decision
someone writes down instead of an omission nobody sees.

NOT IN `just checks` YET, on purpose. It currently fails on exactly eleven
methods, all of them `org.arlen.InstallDaemon1`: `install_package`, `update`,
`install_flatpak`, `uninstall` and `uninstall_flatpak` enqueue their job without
resolving anyone, and the reads beside them (`list_installed`, `preview_upgrade`,
`restore_app`, `list_trashed`, `cleanup_trash`, `get_job_status`) are unresolved
too. Nothing else in the tree is unaccounted for, so that one interface is the
whole of the open question.
Who may install and uninstall is a real decision (the store app, Settings, the
forage CLI, all of which would have to keep working) and not one to make by
picking a list that turns this green. Wire it in once that is answered; until
then run it by hand and read the installd entries as the open item.
"""

import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]

# Both spellings: `#[zbus::interface]` and, where the crate imports it,
# `#[interface]`. Matching only the qualified one made this check parse ZERO
# methods in nine of sixteen files, including installd, while still passing.
IFACE = re.compile(r"#\[(?:zbus::)?interface\b")
# A method of the interface impl: exactly one indent level in.
FN = re.compile(r"^\s{4}(?:pub )?(?:async )?fn (\w+)")
END = re.compile(r"^\s{4}\}\s*$")

# file -> (methods, why they need no caller). Every header-less method must be
# here; anything else fails. Shrinking this list is the good direction.
ACKNOWLEDGED: dict[str, tuple[set[str], str]] = {
    "ai/ai-proxy/src/main.rs": (
        {"list_allowed_providers", "list_providers", "list_provider_usage"},
        "provider catalogue and usage counters, the same for every caller; the "
        "forwarding method that actually spends a key does take the header",
    ),
    "apps/desktop-shell/src-tauri/src/bluetooth_agent.rs": (
        {
            "release",
            "request_pin_code",
            "display_pin_code",
            "request_passkey",
            "display_passkey",
            "request_confirmation",
            "request_authorization",
            "authorize_service",
            "cancel",
        },
        "org.bluez.Agent1: BlueZ calls US on the system bus, so there is no app "
        "caller to resolve; the pairing decision is the user's at the prompt",
    ),
    "daemons/ai-engine-daemon/src/agent_iface.rs": (
        {"status", "working_set"},
        "status is subscribing/idle/busy and working_set is shape-only (the "
        "loaded behaviour names, no user content); completed_actions names the "
        "user's files and IS gated",
    ),
    "apps/desktop-shell/src-tauri/src/sni.rs": (
        {
            "register_status_notifier_host",
            "registered_status_notifier_items",
            "is_status_notifier_host_registered",
            "protocol_version",
        },
        "org.kde.StatusNotifierWatcher, a freedesktop protocol whose registration "
        "is open by specification: any tray-capable app registers itself, and the "
        "signals carry only the bus names it already published",
    ),
    "daemons/installd/install-helper/src/dbus.rs": (
        {"is_installed"},
        "a boolean over a validated app id, the same answer the app directory "
        "already gives a caller that can read it",
    ),
    "daemons/installd/permission-helper/src/dbus.rs": (
        {"profile_exists"},
        "existence of a profile file, no contents; the writers next to it "
        "(WriteProfile, RecordIdentity) all check the caller uid",
    ),
    "daemons/notification-daemon/src/dbus/server.rs": (
        {"close_notification", "get_capabilities", "get_server_information"},
        "org.freedesktop.Notifications: the spec has no caller identity, and any "
        "app may notify and close by id",
    ),
    "daemons/notification-daemon/src/dbus/job_view.rs": (
        {"register", "update", "set_state", "finish", "request_cancel"},
        "org.arlen.JobViewServer1: a progress surface an app keeps for its own "
        "job, holding what that app chose to publish about its own work",
    ),
    "daemons/xdg-portal/daemon/src/interfaces/print.rs": (
        {"version"},
        "the portal interface version constant",
    ),
    "daemons/xdg-portal/daemon/src/interfaces/screenshot.rs": (
        {"version"},
        "the portal interface version constant",
    ),
    "daemons/xdg-portal/daemon/src/interfaces/screencast.rs": (
        {"version", "available_source_types", "available_cursor_modes"},
        "portal capability constants; the methods that actually capture resolve "
        "the caller through the frontend's app id",
    ),
    "daemons/power-daemon/src/dbus.rs": (
        {
            "on_battery",
            "percentage",
            "charge_state",
            "time_to_empty_seconds",
            "time_to_full_seconds",
            "lid_state",
            "profile",
        },
        "read-only power state, published to the session on purpose; the two "
        "actuators (suspend, set_profile) both resolve the caller and check its "
        "system.power grant",
    ),
}


def interface_methods(text: str) -> list[tuple[str, bool]]:
    """Every METHOD of every interface impl, with whether it sees the header.

    Signals are excluded rather than acknowledged one by one: a `#[zbus(signal)]`
    is emitted BY the daemon, so asking who called it is meaningless. They were in
    the acknowledged list at first, with reasons that all amounted to "this is a
    signal", which is the check failing to model its own subject.
    """
    out: list[tuple[str, bool]] = []
    for m in IFACE.finditer(text):
        start = text.index("{", m.end())
        depth, i = 1, start + 1
        while i < len(text) and depth:
            depth += (text[i] == "{") - (text[i] == "}")
            i += 1
        lines = text[start + 1 : i - 1].split("\n")
        # Attributes are carried forward to the fn they precede and cleared at it,
        # rather than read from a fixed window above. A window that happens to
        # reach back over the PREVIOUS item's attributes would silently drop a
        # method that follows a signal, and a check that quietly stops asking
        # about something is the failure this file is here to prevent.
        pending_signal = False
        for idx, line in enumerate(lines):
            stripped = line.strip()
            if stripped.startswith("#["):
                pending_signal = pending_signal or "zbus(signal)" in stripped
                continue
            fm = FN.match(line)
            if not fm:
                # A doc comment or blank line keeps the attributes pending; any
                # other content means they belonged to something already passed.
                if stripped and not stripped.startswith("//"):
                    pending_signal = False
                continue
            was_signal = pending_signal
            pending_signal = False
            if was_signal:
                continue
            j, body = idx + 1, []
            while j < len(lines) and not END.match(lines[j]):
                body.append(lines[j])
                j += 1
            out.append((fm.group(1), "zbus(header)" in "\n".join(body)))
    return out


def main() -> int:
    # Selected by the ATTRIBUTE, not by any mention of zbus: a file that merely
    # imports `interface` is not an interface, and a file that imports it and then
    # uses the short form must not be missed.
    files = subprocess.run(
        ["git", "grep", "-lE", r"#\[(zbus::)?interface"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()

    problems: list[str] = []
    checked = blind = 0
    for f in sorted(files):
        methods = interface_methods((ROOT / f).read_text())
        if not methods:
            continue
        allowed, _ = ACKNOWLEDGED.get(f, (set(), ""))
        seen_blind = set()
        for name, has_header in methods:
            checked += 1
            if has_header:
                continue
            blind += 1
            seen_blind.add(name)
            if name not in allowed:
                problems.append(
                    f"{f}: {name} takes no #[zbus(header)], so it cannot know its caller. "
                    "Take the header and gate it, or list it with the reason it is safe."
                )
        for stale in sorted(allowed - seen_blind):
            problems.append(
                f"{f}: {stale} is listed as needing no caller but now takes the header "
                "(or is gone). Delete the entry: the list is meant to shrink."
            )

    for f in sorted(set(ACKNOWLEDGED) - set(files)):
        problems.append(f"{f} is listed but has no zbus interface any more; delete the entry")

    if problems:
        print("D-Bus methods that cannot see their caller:\n")
        for p in problems:
            print(f"  - {p}")
        return 1

    print(
        f"{checked} D-Bus method(s) across {len(files)} file(s); "
        f"{blind} take no caller and each is acknowledged"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
