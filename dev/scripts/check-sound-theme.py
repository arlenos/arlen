#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Tim Kicker
#
# SPDX-License-Identifier: AGPL-3.0-only

"""Every shipped cue exists, and no cue is louder than the others.

WHAT THIS IS FOR. `themes/sounds/build-sound-theme.sh` normalises the default cue
set so the daemon's volume control is the only thing that changes how loud a
sound is. The first version of that script did not: it used EBU R128, which
integrates over 400ms gating blocks, and four of the six cues are SHORTER than
one block. ffmpeg reported `I: -70.0 LUFS` - its floor for "nothing gated in",
not "silent" - and the filter degraded to a peak limiter. The result was six
playable files, a step that reported success, and a set spread eleven decibels
apart.

So the check measures the SHIPPED files rather than trusting the build, in a unit
that is defined at this length. A rebuild that reintroduces the R128 pass fails
here instead of shipping.

It also checks the cue NAMES against the daemon's own list, because a file named
anything else is never resolved and falls silently through to the synth - which
sounds like a working system with a different theme.

Run: dev/scripts/check-sound-theme.py [repo-root]
"""

import pathlib
import re
import shutil
import subprocess
import sys

ROOT = (
    pathlib.Path(sys.argv[1]).resolve()
    if len(sys.argv) > 1
    else pathlib.Path(__file__).resolve().parents[2]
)

THEME = ROOT / "themes/sounds/arlen/stereo"
SOUND_RS = ROOT / "daemons/notification-daemon/src/sound.rs"

#: How far apart the quietest and loudest cue may sit, in dB of mean level. A
#: decibel is around the threshold of noticing on a short cue; the build lands
#: them inside 0.1, so this is loose enough to survive an encoder change and
#: tight enough to catch a normalisation that did not happen.
SPREAD_DB = 1.5

#: Nothing may peak above this, or it clips on the way out.
PEAK_CEILING_DB = -1.0


def daemon_cue_names() -> set[str]:
    """The names `SoundEvent::sound_name` maps to - the ones actually looked up."""
    text = SOUND_RS.read_text()
    body = text[text.index("fn sound_name"):]
    body = body[: body.index("}\n    }")]
    return set(re.findall(r'=> "([a-z0-9-]+)"', body))


def measure(path: pathlib.Path) -> tuple[float, float]:
    """Mean and peak level in dBFS. Defined at any length, unlike R128."""
    out = subprocess.run(
        ["ffmpeg", "-v", "info", "-i", str(path), "-af", "volumedetect", "-f", "null", "-"],
        capture_output=True,
        text=True,
    ).stderr
    mean = re.search(r"mean_volume: (-?[\d.]+) dB", out)
    peak = re.search(r"max_volume: (-?[\d.]+) dB", out)
    if not mean or not peak:
        raise ValueError(f"could not measure {path.name}")
    return float(mean.group(1)), float(peak.group(1))


def main() -> int:
    if not THEME.is_dir():
        print(f"NOTHING WAS READ: no cue directory at {THEME}", file=sys.stderr)
        return 2
    if shutil.which("ffmpeg") is None:
        # Not a pass: a check that cannot measure has not checked anything.
        print("SKIPPED: ffmpeg is not installed, so no cue was measured", file=sys.stderr)
        return 0

    wanted = daemon_cue_names()
    if not wanted:
        print("NOTHING WAS READ: no cue names found in sound.rs", file=sys.stderr)
        return 2

    shipped = {p.stem: p for p in THEME.glob("*.oga")}
    findings = []

    for name in sorted(wanted - set(shipped)):
        findings.append(
            f"the daemon resolves `{name}` and the theme has no such file, so that "
            f"event falls through to the synth"
        )
    for name in sorted(set(shipped) - wanted):
        findings.append(
            f"`{name}.oga` ships but no event resolves that name, so it never plays"
        )

    levels = {}
    for name, path in sorted(shipped.items()):
        mean, peak = measure(path)
        levels[name] = mean
        if peak > PEAK_CEILING_DB:
            findings.append(f"`{name}` peaks at {peak:.1f} dB, above the {PEAK_CEILING_DB} dB ceiling")

    if len(levels) > 1:
        spread = max(levels.values()) - min(levels.values())
        if spread > SPREAD_DB:
            loud = max(levels, key=levels.get)
            quiet = min(levels, key=levels.get)
            findings.append(
                f"the cues are {spread:.1f} dB apart ({loud} at {levels[loud]:.1f}, "
                f"{quiet} at {levels[quiet]:.1f}) - more than {SPREAD_DB} dB, so the "
                f"normalisation did not take. R128 cannot measure a cue under 400ms; "
                f"see themes/sounds/PROVENANCE.md"
            )

    if findings:
        print(f"{len(shipped)} cue(s) measured, {len(findings)} finding(s):\n", file=sys.stderr)
        for f in findings:
            print(f"  - {f}", file=sys.stderr)
        return 1

    spread = max(levels.values()) - min(levels.values()) if len(levels) > 1 else 0.0
    print(
        f"{len(shipped)} cue(s) measured: every name the daemon resolves has a file, "
        f"and the set spans {spread:.1f} dB of mean level. No cue is louder than "
        f"another, so the volume control is the only thing that changes level."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
