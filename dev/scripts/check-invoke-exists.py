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
    the end but never fails the check, because a shared helper in `ui-kit` can call
    a command the app's own source never names (`frontend_log` is the obvious one),
    and a scanner that only reads `apps/*/src` cannot see it.
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

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

INVOKE = re.compile(r'invoke(?:<[^>]*>)?\(\s*["\'`]([A-Za-z_][A-Za-z0-9_]*)["\'`]')
HANDLER = re.compile(r"generate_handler!\s*\[(.*?)\]", re.S)

# The commands with no host, as of 9 August, with what each one is. Keeping the
# reason next to the name is the difference between an inventory and an alibi.
KNOWN: dict[str, dict[str, str]] = {
    "knowledge": {
        "knowledge_capsule_mint": "the capsule mint button",
        "knowledge_capsule_preview": "the capsule preview",
        "knowledge_capsule_revoke": "revoking a minted capsule",
        "knowledge_capsules": "the capsule list",
        "knowledge_library": "the library view",
        "knowledge_search_save": "saving a search",
        "knowledge_timeline_delete": "deleting a timeline entry",
        "knowledge_timeline_pause": "pausing collection",
        "open_settings_route": "the deep link into Settings",
    },
    "settings": {
        "browse_bottle_files": "the Windows-app bottle browser",
        "clear_bottle_caches": "bottle cache clearing",
        "delete_bottle": "deleting a bottle",
        "install_windows_app": "installing a Windows app",
        "list_bottles": "the bottle list",
        "set_bottle_config": "bottle settings",
        "set_windows_defaults": "the Windows-app defaults",
        "print_job_retry": "retrying a print job",
        "printers_add": "adding a printer",
        "printers_discover": "printer discovery",
        "printers_remove": "removing a printer",
        "sentinel_fix_posture": "the security posture fix",
        "sentinel_get_state": "the sentinel state",
        "sentinel_set_alerts": "sentinel alerts",
        "sentinel_set_detector": "a sentinel detector toggle",
        "sentinel_set_sensitivity": "sentinel sensitivity",
        "topbar_items": "the top-bar item list",
    },
    "desktop-shell": {
        "cancel_print": "cancelling a print job",
        "poll_print_request": "the print request poll",
        "submit_print": "submitting a print job",
        "cancel_screencast": "cancelling a screencast",
        "start_screencast": "starting a screencast",
        "stop_capture": "stopping a capture",
        "capture_status": "the capture indicator",
        "list_capture_sources": "the capture source picker",
        "dictation_status": "the dictation indicator",
        "stop_dictation": "stopping dictation",
        "get_module_errors": "module error reporting",
        "list_modules": "the module list",
        "list_jobs": "the jobs zone",
        "waypointer_ask": "asking the assistant from the launcher",
        "windows_file_install": "installing a Windows file",
        "windows_file_request": "the Windows file prompt",
        "windows_file_run": "running a Windows file",
    },
    "store": {
        "store_uninstall": "uninstalling an app",
        "store_update": "updating one app",
        "store_update_all_routine": "update all",
    },
    "text-editor": {
        "Authorize": "the AI-edit gate call",
        "ai_edit": "proposing an assistant edit",
        "open_file": "opening a file from the lens",
        "project_of": "the lens project section",
        "provenance_of": "the lens provenance section",
        "related_of": "the lens backlinks",
    },
    "harness": {
        "register_menu": "the app menu (the plugin's name for it is menu_register)",
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
    apps = sorted(p for p in (ROOT / "apps").iterdir() if (p / "package.json").exists())

    for app in apps:
        calls: set[str] = set()
        src = app / "src"
        if src.exists():
            for f in list(src.rglob("*.ts")) + list(src.rglob("*.svelte")):
                calls |= set(INVOKE.findall(f.read_text(encoding="utf-8", errors="replace")))
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
                f"apps/{app.name}: invokes `{name}`, which neither its host nor a "
                f"plugin registers. Every call throws; whatever the catch does is "
                f"what the user gets."
            )
        for name in sorted(handlers - calls):
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
    print(
        f"\n{len(uncalled)} registered command(s) nothing under apps/*/src invokes. "
        f"Informational only: a ui-kit helper can call one this scanner cannot see."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
