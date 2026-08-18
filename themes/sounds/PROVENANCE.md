# Where the default cue set came from

Not for anyone's lawyer. This is here so that in a year the answer to "what is
this sound and why does it sound like that" is a paragraph rather than an
afternoon.

## The pack

**Kenney `digital-audio`**, fetched 18 Aug 2026 from
<https://kenney.nl/assets/digital-audio>.

- File: `kenney_digital-audio.zip`, 990367 bytes
- sha256: `24e6ce28b76a6d8c89cff4d331e0965ff5c3de8a73c612028e9d363cc64e4f06`
- Licence: **CC0 1.0**, stated in the pack's own `License.txt`: "You may use
  these assets in personal and commercial projects. Credit (Kenney or
  www.kenney.nl) would be nice but is not mandatory." We credit it here.

The hash is pinned in `build-sound-theme.sh`, so a re-run against a re-uploaded
pack fails loudly instead of quietly producing a different theme.

**One pack, no mixing.** Every cue is from `digital-audio`. The plan is explicit
that cross-pack mixing breaks the sonic family, and it is the kind of thing that
is invisible in a diff and obvious in the ears.

## The mapping

Cue names come from `SoundEvent::sound_name` in
`daemons/notification-daemon/src/sound.rs` - what the daemon actually looks up,
rather than from any prose list. Six events, six files.

| cue | source | why |
|---|---|---|
| `message-new-instant` | `phaserUp5.ogg` | something arrived: a rising shape |
| `dialog-error` | `phaserDown2.ogg` | something failed: falling |
| `dialog-warning` | `phaseJump2.ogg` | attention without failure |
| `complete` | `powerUp5.ogg` | finished well: rising, more body |
| `device-added` | `phaserUp2.ogg` | attached: rising, sibling of the above |
| `device-removed` | `phaserDown1.ogg` | detached: falling, its mirror |

The mapping is **structural** - rise for arrival and completion, fall for failure
and departure, which is what the freedesktop names mean. Whether these six sound
like a family is a listening judgement and was deliberately not made here.

## The processing

Each cue is level-matched to **-22 dB mean** and limited at -2 dBFS, then encoded
to Ogg Vorbis (`-q:a 5`). Measured result, all six: -22.0 dB mean (one at -21.9),
peaks between -3.5 and -9.0 dB.

**Why RMS and not EBU R128, which the plan's "loudness-normalise" implies.** The
first cut used `loudnorm=I=-23` and it silently did almost nothing to half the
set. R128 integrates over 400 ms gating blocks and four of these cues are shorter
than one block, so there is no measurement to act on - ffmpeg reports
`I: -70.0 LUFS`, which is its floor for "nothing gated in" and not "silent", and
the filter degrades to little more than a peak limiter. The three cues over
400 ms came out at -26 dB mean and the three under it at -15 to -20 dB: **seven
to eleven decibels apart**, in a set whose entire purpose is that no cue is louder
than another. Six playable files, and the step had reported success.

Mean volume is defined at any length, so that is what the gain is computed from
now, and `dev/scripts/check-sound-theme.py` re-measures the shipped files so the
same silence cannot come back.

## Two things a listening pass has to settle

1. **Length.** The plan asks for 50-200 ms cues. **Nothing in this pack is that
   short**: the briefest is 313 ms and the set runs to 470 ms. The tails are not
   padding - `silencedetect` at -50 dB finds no trailing silence to trim, so
   cutting to 200 ms would cut audible decay and change how each cue sounds. That
   is a taste decision, so the cues ship at their natural length and the conflict
   is recorded here rather than resolved quietly.

2. **The set itself.** It is coherent by construction, not by ear. A notification
   tone has to survive its tenth repetition in an afternoon, which no measurement
   here can tell you.
