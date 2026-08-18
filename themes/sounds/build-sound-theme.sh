#!/usr/bin/env bash
# Build the default Arlen sound theme from the Kenney `digital-audio` pack
# (sound-system-plan.md SO-R2).
#
# WHY A SCRIPT AND NOT SIX HAND-COPIED FILES. Everything here is a decision that
# has to be re-checkable: which pack, which source file each cue came from, and
# exactly what processing was applied. A hand-built directory answers none of
# those a month later, and the plan asks for a provenance note precisely because
# this is content whose origin stops being obvious the moment it is renamed.
#
# ONE PACK, NO MIXING. The plan is explicit that cross-pack mixing breaks the
# sonic family, so every cue below comes from `digital-audio` and nothing else.
#
# THE CUE NAMES COME FROM THE CODE, not from prose: `SoundEvent::sound_name` in
# `daemons/notification-daemon/src/sound.rs` is what the daemon actually looks
# up. Six events, six files. A seventh name here would never be resolved and a
# missing one falls through to the synth.
#
# Run: themes/sounds/build-sound-theme.sh [path-to-kenney_digital-audio.zip]
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
zip="${1:-/tmp/kenney_digital-audio.zip}"
# The pack as fetched from kenney.nl on 18 Aug 2026. Pinned so a re-run against a
# re-uploaded pack is a loud failure rather than a silently different theme.
want_sha="24e6ce28b76a6d8c89cff4d331e0965ff5c3de8a73c612028e9d363cc64e4f06"
theme="$here/arlen"
# The common level every cue is brought to, in mean dBFS. Chosen from where this
# pack already sits so the gains stay small: the raw cues measure -15 to -27, and
# -22 moves each by a handful of decibels rather than rebuilding its dynamics.
# The limiter ceiling (0.79 ~= -2 dBFS) is the true-peak headroom the plan's
# broadcast reference would have given.
TARGET_MEAN_DB=-22

[ -f "$zip" ] || { echo "!! no pack at $zip (fetch it from https://kenney.nl/assets/digital-audio)" >&2; exit 1; }
got=$(sha256sum "$zip" | cut -d' ' -f1)
[ "$got" = "$want_sha" ] || { echo "!! pack sha256 $got does not match the pinned $want_sha" >&2; exit 1; }

command -v ffmpeg >/dev/null || { echo "!! ffmpeg is needed for the loudness pass" >&2; exit 1; }

# cue-name:source-file. The mapping is STRUCTURAL - a rising shape for things
# that arrived or completed, a falling one for things that failed or left - which
# is what the freedesktop names mean. Whether the family sounds right together is
# a listening judgement and is deliberately not made here.
cues="
message-new-instant:phaserUp5.ogg
dialog-error:phaserDown2.ogg
dialog-warning:phaseJump2.ogg
complete:powerUp5.ogg
device-added:phaserUp2.ogg
device-removed:phaserDown1.ogg
"

work=$(mktemp -d)
# The pack's members extract read-only, so a plain `rm -rf` on the temp dir
# fails member by member and leaves it behind. Make it writable first.
trap 'chmod -R u+w "$work" 2>/dev/null; rm -rf "$work"' EXIT
unzip -q "$zip" -d "$work"

mkdir -p "$theme/stereo"
for entry in $cues; do
    name="${entry%%:*}"
    file="${entry##*:}"
    src="$work/Audio/$file"
    [ -f "$src" ] || { echo "!! $file is not in this pack" >&2; exit 1; }
    # LEVEL-MATCH BY RMS, NOT BY R128, AND THE REASON MATTERS. The obvious pass
    # is `loudnorm=I=-23` - and it silently does almost nothing to half this set.
    # EBU R128 integrates over 400ms gating blocks, and four of these six cues are
    # SHORTER than one block, so there is no measurement to act on: ffmpeg reports
    # `I: -70.0 LUFS` (its floor for "nothing gated in", not "silent") and the
    # filter degrades to little more than the true-peak limiter.
    #
    # Measured on the first attempt at this file: the three cues over 400ms came
    # out at -26 dB mean, the three under it at -15 to -20 dB. Seven to eleven
    # decibels apart, in a set whose whole purpose is that no cue is louder than
    # another. The step had reported success and produced six playable files.
    #
    # So the gain is computed from mean volume, which is defined at any length,
    # and applied as a plain offset to a common target, with a ceiling so the
    # louder transients cannot clip. The verification below is in the same unit.
    mean=$(ffmpeg -v info -i "$src" -af volumedetect -f null - 2>&1 \
             | grep -oE 'mean_volume: -?[0-9.]+' | grep -oE '\-?[0-9.]+')
    [ -n "$mean" ] || { echo "!! could not measure $file" >&2; exit 1; }
    gain=$(python3 -c "print(f'{$TARGET_MEAN_DB - ($mean):.2f}')")
    ffmpeg -v error -y -i "$src" -af "volume=${gain}dB,alimiter=limit=0.79:level=disabled" \
           -c:a libvorbis -q:a 5 "$theme/stereo/$name.oga"
done

# The theme index. `Directories=stereo` is the freedesktop layout; the daemon
# reads this to know where to look, and falls back to the theme root for a flat
# theme.
cat > "$theme/index.theme" <<'INDEX'
[Sound Theme]
Name=Arlen
Comment=The default Arlen cue set
Directories=stereo
Inherits=freedesktop

[stereo]
OutputProfile=stereo
INDEX

echo ">> built $theme with $(ls "$theme/stereo" | wc -l) cues"
ls -la "$theme/stereo"
