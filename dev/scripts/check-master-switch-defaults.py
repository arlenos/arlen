#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only
"""The AI master switches ship from two places, and they must agree.

The config-broker is the separate-uid owner of `enabled`, `access_level`,
`executor_live`, `action_mode` and `provider`. A fresh store is seeded from
`AiMasterSwitches::shipped_default()`, and that function's own doc names the invariant
this enforces: it "matches the shipped `ai.toml` (`DEFAULT_AI` in the settings
app) ... The two must stay in step until the cutover makes the broker the sole
owner of these defaults."

Nothing checked it. Until the cutover finishes, both files ship, and which one a
machine obeys depends on whether the broker is up: `engine_config::executor_live`
asks the broker FIRST and falls back to `ai.toml` only when it cannot be reached.
So a disagreement does not resolve to one answer or the other, it resolves to
whichever daemon happened to start - and the switches in question decide whether
the assistant may act at all. That is the worst shape a divergence can take, and
it is why this compares literals rather than trusting the two authors to
remember each other.

NOT CHECKED HERE, deliberately: `dev/mkosi/.../home/arlen/.config/arlen/ai.toml`.
That file is a dogfood machine's deliberate configuration, not a shipped default,
and it diverges on purpose. It is also where the consequence shows: its four
switches are silently overridden by the broker's seed on first boot, because the
broker runs as root and the migration that would read them looks in root's home.
Whether the image should keep that configuration, and how, is a deployment
decision recorded in `coder-reports.md` rather than something a gate should
settle.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BROKER = ROOT / "daemons/config-broker/src/state.rs"
SETTINGS = ROOT / "apps/settings/src-tauri/src/commands/config.rs"


def broker_shipped() -> dict[str, str]:
    """The literal field values of `AiMasterSwitches::shipped_default()`."""
    text = BROKER.read_text()
    m = re.search(r"fn shipped_default\(\)[^{]*\{(.*?)\n    \}", text, re.S)
    if not m:
        sys.exit("check-master-switch-defaults: no `fn shipped_default()` in state.rs")
    out = {}
    for field, value in re.findall(r"(\w+):\s*([^,\n]+),", m.group(1)):
        v = value.strip().strip('"').replace('.to_string()', '').strip('"')
        v = v.replace("ActionMode::", "").lower()
        out[field] = v
    return out


def default_ai() -> dict[str, str]:
    """The `[ai]` and `[agent]` switches of the settings app's `DEFAULT_AI`."""
    text = SETTINGS.read_text()
    m = re.search(r'const DEFAULT_AI: &str = r##"(.*?)"##', text, re.S)
    if not m:
        sys.exit("check-master-switch-defaults: no DEFAULT_AI in config.rs")
    doc, section, out = m.group(1), "", {}
    for line in doc.splitlines():
        line = line.strip()
        if line.startswith("["):
            section = line.strip("[]")
            continue
        if section not in ("ai", "agent") or "=" not in line or line.startswith("#"):
            continue
        k, _, v = line.partition("=")
        out[k.strip()] = v.strip().strip('"').lower()
    return out


# The switches the broker owns. A name absent from DEFAULT_AI is not a mismatch:
# the shipped ai.toml omits the ones whose safe value is the type's own default,
# so absence means "the floor", which is what the broker seeds too. The pairs are
# listed rather than derived so adding a sixth switch has to be a decision here.
OWNED = {
    "enabled": "false",
    "access_level": "3",
    "executor_live": "false",
    "action_mode": "suggest",
    "provider": "ollama-default",
}


def main() -> int:
    shipped, default = broker_shipped(), default_ai()
    bad = []
    for name, floor in OWNED.items():
        b = shipped.get(name)
        if b is None:
            bad.append(f"`{name}` is not set by AiMasterSwitches::shipped_default()")
            continue
        d = default.get(name, floor)
        if b != d:
            bad.append(
                f"`{name}`: the broker seeds {b!r}, the shipped ai.toml says {d!r}. "
                f"Until the cutover both ship, and which one a machine obeys "
                f"depends on whether the broker is up."
            )
    if bad:
        print("the AI master switches disagree between the two places that ship them:")
        for b in bad:
            print(f"  - {b}")
        print()
        print("  Both are read: `engine_config::executor_live` asks the broker first")
        print("  and falls back to ai.toml when it cannot be reached, so a difference")
        print("  here is a switch whose value depends on a daemon's liveness.")
        print("  `AiMasterSwitches::shipped_default()` names this invariant in its own doc.")
        return 1
    print(
        f"{len(OWNED)} AI master switch(es) ship the same value from the broker's "
        f"seed and the settings app's DEFAULT_AI."
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
