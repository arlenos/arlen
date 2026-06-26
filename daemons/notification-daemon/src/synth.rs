//! SO-R4: the deterministic parametric synth engine (`sound-system-plan.md`). A
//! theme can declare a sound cue as synthesis tokens (an oscillator + a frequency
//! sweep + amplitude modulation + an ADSR envelope + a touch of reverb) instead of a
//! sample file - the zero-asset, zero-licence "alternative" sound-theme class, a
//! sound palette in the TOML the way the colour tokens are.
//!
//! The chain is the shape the research settled on (BeepBank-500, the deep-research
//! synthesis): `oscillator -> AM -> ADSR -> RMS-normalize -> reverb`, deterministic
//! so the same tokens always render the same cue. It is pure computation - it
//! produces the f32 PCM the daemon's playback path then sends to PipeWire - so it is
//! fully testable without an audio device. Honestly scoped: the research found
//! sampled auditory icons beat synth earcons, so this is the lightweight alternative,
//! never the default and never sold as "premium".

use serde::Deserialize;
use std::f32::consts::{PI, TAU};

/// The oscillator waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Waveform {
    /// A pure sine - the calmest, the default.
    #[default]
    Sine,
    /// A square wave (hollow, retro).
    Square,
    /// A triangle wave (soft, between sine and square).
    Triangle,
    /// A sawtooth ramp (buzzy, bright).
    Saw,
}

impl Waveform {
    /// One sample of the waveform at `phase` radians; the caller accumulates phase,
    /// so this only needs the wrapped phase. Range is `[-1.0, 1.0]`.
    fn sample(self, phase: f32) -> f32 {
        let p = phase.rem_euclid(TAU);
        match self {
            Waveform::Sine => p.sin(),
            Waveform::Square => {
                if p < PI {
                    1.0
                } else {
                    -1.0
                }
            }
            // A rising ramp from -1 to 1 across the period.
            Waveform::Saw => p / PI - 1.0,
            // -1 -> 1 -> -1 across the period.
            Waveform::Triangle => {
                let x = p / PI; // 0..2
                if x < 1.0 {
                    -1.0 + 2.0 * x
                } else {
                    3.0 - 2.0 * x
                }
            }
        }
    }
}

/// The ADSR amplitude envelope. Times are in milliseconds; `sustain` is a 0..1
/// level the note holds at after the decay, until the release tail.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Adsr {
    /// Rise time from silence to full (ms).
    #[serde(default = "default_attack_ms")]
    pub attack_ms: f32,
    /// Fall time from full to the sustain level (ms).
    #[serde(default = "default_decay_ms")]
    pub decay_ms: f32,
    /// The held level after decay (0..1).
    #[serde(default = "default_sustain")]
    pub sustain: f32,
    /// Fall time from the sustain level back to silence (ms).
    #[serde(default = "default_release_ms")]
    pub release_ms: f32,
}

fn default_attack_ms() -> f32 {
    5.0
}
fn default_decay_ms() -> f32 {
    40.0
}
fn default_sustain() -> f32 {
    0.5
}
fn default_release_ms() -> f32 {
    80.0
}

impl Default for Adsr {
    fn default() -> Self {
        Self {
            attack_ms: default_attack_ms(),
            decay_ms: default_decay_ms(),
            sustain: default_sustain(),
            release_ms: default_release_ms(),
        }
    }
}

impl Adsr {
    /// The envelope amplitude (0..1) at `t_ms` for a cue of `dur_ms`. Non-negative,
    /// and 0 outside `[0, dur_ms]`. If the attack+decay+release do not fit the
    /// duration (a very short cue) the phases are clamped in order, so the envelope
    /// stays well-formed (it never goes negative or above the peak).
    fn amplitude(&self, t_ms: f32, dur_ms: f32) -> f32 {
        if t_ms < 0.0 || t_ms > dur_ms {
            return 0.0;
        }
        let sustain = self.sustain.clamp(0.0, 1.0);
        let release_start = (dur_ms - self.release_ms.max(0.0)).max(0.0);
        if t_ms < self.attack_ms && self.attack_ms > 0.0 {
            // Attack: 0 -> 1.
            (t_ms / self.attack_ms).clamp(0.0, 1.0)
        } else if t_ms < self.attack_ms + self.decay_ms && self.decay_ms > 0.0 {
            // Decay: 1 -> sustain.
            let into = (t_ms - self.attack_ms) / self.decay_ms;
            (1.0 - (1.0 - sustain) * into).clamp(sustain, 1.0)
        } else if t_ms < release_start {
            // Sustain hold.
            sustain
        } else if self.release_ms > 0.0 {
            // Release: sustain -> 0.
            let into = (t_ms - release_start) / self.release_ms;
            (sustain * (1.0 - into)).max(0.0)
        } else {
            0.0
        }
    }
}

/// A synthesised cue declared entirely by parameters (a theme's `[sounds.synth.<event>]`
/// token block). Every field has a sensible default so a sparse declaration still
/// renders a usable cue.
#[derive(Debug, Clone, Deserialize)]
pub struct SynthParams {
    /// The oscillator shape.
    #[serde(default)]
    pub waveform: Waveform,
    /// The starting frequency in Hz.
    #[serde(default = "default_freq_hz")]
    pub freq_hz: f32,
    /// The ending frequency in Hz; a linear sweep from `freq_hz` to this over the
    /// cue (a "blip up" or "blip down"). Defaults equal to `freq_hz` (no sweep).
    #[serde(default)]
    pub freq_end_hz: Option<f32>,
    /// Amplitude-modulation rate in Hz (a tremolo); 0 disables AM.
    #[serde(default)]
    pub am_hz: f32,
    /// AM depth 0..1 (how deep the tremolo dips).
    #[serde(default)]
    pub am_depth: f32,
    /// Total cue length in milliseconds.
    #[serde(default = "default_duration_ms")]
    pub duration_ms: f32,
    /// The amplitude envelope.
    #[serde(default)]
    pub adsr: Adsr,
    /// The target RMS the cue is normalised to (0..1) so cues are loudness-consistent.
    #[serde(default = "default_rms")]
    pub rms: f32,
    /// Reverb wet mix 0..1; 0 disables the reverb tail.
    #[serde(default)]
    pub reverb_mix: f32,
    /// Reverb feedback decay 0..1 (how long the tail rings); only used when mixed in.
    #[serde(default = "default_reverb_decay")]
    pub reverb_decay: f32,
    /// Reverb delay in milliseconds (the comb spacing).
    #[serde(default = "default_reverb_delay_ms")]
    pub reverb_delay_ms: f32,
}

fn default_freq_hz() -> f32 {
    660.0
}
fn default_duration_ms() -> f32 {
    180.0
}
fn default_rms() -> f32 {
    0.2
}
fn default_reverb_decay() -> f32 {
    0.4
}
fn default_reverb_delay_ms() -> f32 {
    35.0
}

impl Default for SynthParams {
    fn default() -> Self {
        Self {
            waveform: Waveform::default(),
            freq_hz: default_freq_hz(),
            freq_end_hz: None,
            am_hz: 0.0,
            am_depth: 0.0,
            duration_ms: default_duration_ms(),
            adsr: Adsr::default(),
            rms: default_rms(),
            reverb_mix: 0.0,
            reverb_decay: default_reverb_decay(),
            reverb_delay_ms: default_reverb_delay_ms(),
        }
    }
}

/// Render `params` to mono f32 PCM at `sample_rate`. Deterministic - identical
/// params always produce identical samples. The output is RMS-normalised to
/// `params.rms` and peak-guarded so it never clips `[-1.0, 1.0]`, and it never
/// contains a NaN (degenerate params fall back to silence rather than poison the
/// buffer).
pub fn synthesize(params: &SynthParams, sample_rate: u32) -> Vec<f32> {
    let sr = sample_rate.max(1) as f32;
    let n = ((params.duration_ms.max(0.0) / 1000.0) * sr).round() as usize;
    if n == 0 {
        return Vec::new();
    }
    let f0 = params.freq_hz.max(0.0);
    let f1 = params.freq_end_hz.unwrap_or(params.freq_hz).max(0.0);
    let am_depth = params.am_depth.clamp(0.0, 1.0);

    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    let mut am_phase = 0.0f32;
    for i in 0..n {
        let frac = i as f32 / n as f32;
        // Oscillator with a linear frequency sweep.
        let freq = f0 + (f1 - f0) * frac;
        let mut s = params.waveform.sample(phase);
        phase += TAU * freq / sr;

        // AM (tremolo): scale by 1 - depth*(0.5 - 0.5*cos) so it dips, not boosts.
        if params.am_hz > 0.0 && am_depth > 0.0 {
            let m = 0.5 - 0.5 * am_phase.cos();
            s *= 1.0 - am_depth * m;
            am_phase += TAU * params.am_hz / sr;
        }

        // ADSR.
        let t_ms = frac * params.duration_ms;
        s *= params.adsr.amplitude(t_ms, params.duration_ms);
        out.push(s);
    }

    rms_normalize(&mut out, params.rms.clamp(0.0, 1.0));
    if params.reverb_mix > 0.0 {
        apply_reverb(
            &mut out,
            params.reverb_mix.clamp(0.0, 1.0),
            params.reverb_decay.clamp(0.0, 0.95),
            params.reverb_delay_ms.max(1.0),
            sr,
        );
    }
    peak_guard(&mut out);
    sanitize(&mut out);
    out
}

/// Scale the buffer so its RMS equals `target`. A near-silent buffer is left as is
/// (no divide by ~zero).
fn rms_normalize(buf: &mut [f32], target: f32) {
    if buf.is_empty() || target <= 0.0 {
        return;
    }
    let sum_sq: f32 = buf.iter().map(|x| x * x).sum();
    let rms = (sum_sq / buf.len() as f32).sqrt();
    if rms < 1e-6 {
        return;
    }
    let gain = target / rms;
    for x in buf.iter_mut() {
        *x *= gain;
    }
}

/// A single feedback-comb reverb tail (`y[i] = x[i] + decay * y[i - delay]`), mixed
/// dry/wet. Honest and modest - one comb, not a hall.
fn apply_reverb(buf: &mut [f32], mix: f32, decay: f32, delay_ms: f32, sr: f32) {
    let delay = ((delay_ms / 1000.0) * sr).round() as usize;
    if delay == 0 || buf.is_empty() {
        return;
    }
    let mut wet = vec![0.0f32; buf.len()];
    for i in 0..buf.len() {
        let fb = if i >= delay { wet[i - delay] * decay } else { 0.0 };
        wet[i] = buf[i] + fb;
    }
    for i in 0..buf.len() {
        buf[i] = (1.0 - mix) * buf[i] + mix * wet[i];
    }
}

/// Scale down so the peak magnitude never exceeds 0.99 (never clips).
fn peak_guard(buf: &mut [f32]) {
    let peak = buf.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 0.99 {
        let gain = 0.99 / peak;
        for x in buf.iter_mut() {
            *x *= gain;
        }
    }
}

/// Replace any non-finite sample with 0 so a degenerate parameter set can never
/// emit a NaN/inf into the playback path.
fn sanitize(buf: &mut [f32]) {
    for x in buf.iter_mut() {
        if !x.is_finite() {
            *x = 0.0;
        }
    }
}

/// Encode mono f32 PCM (`[-1, 1]`) as a 16-bit little-endian WAV file, so a
/// synthesised cue is a playable artifact the daemon's decode/play path can treat
/// like any sample. Self-contained (no encoder dependency).
pub fn to_wav16(pcm: &[f32], sample_rate: u32) -> Vec<u8> {
    let bits = 16u16;
    let channels = 1u16;
    let byte_rate = sample_rate * channels as u32 * (bits / 8) as u32;
    let block_align = channels * (bits / 8);
    let data_len = (pcm.len() * 2) as u32;
    let mut v = Vec::with_capacity(44 + pcm.len() * 2);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVE");
    v.extend_from_slice(b"fmt ");
    v.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&sample_rate.to_le_bytes());
    v.extend_from_slice(&byte_rate.to_le_bytes());
    v.extend_from_slice(&block_align.to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    for &s in pcm {
        let q = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        v.extend_from_slice(&q.to_le_bytes());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    #[test]
    fn waveforms_stay_in_range() {
        for wf in [Waveform::Sine, Waveform::Square, Waveform::Triangle, Waveform::Saw] {
            for k in 0..1000 {
                let p = k as f32 * 0.013;
                let s = wf.sample(p);
                assert!((-1.0..=1.0).contains(&s), "{wf:?} out of range at {p}: {s}");
            }
        }
    }

    #[test]
    fn length_matches_duration() {
        let p = SynthParams {
            duration_ms: 100.0,
            ..SynthParams::default()
        };
        let pcm = synthesize(&p, SR);
        // 100ms at 48k = 4800 samples.
        assert_eq!(pcm.len(), 4800);
    }

    #[test]
    fn deterministic() {
        let p = SynthParams::default();
        assert_eq!(synthesize(&p, SR), synthesize(&p, SR), "same params must render identically");
    }

    #[test]
    fn never_clips_and_is_finite() {
        // An adversarial-loud, sweeping, reverbed cue must still be bounded + finite.
        let p = SynthParams {
            waveform: Waveform::Saw,
            freq_hz: 220.0,
            freq_end_hz: Some(1760.0),
            am_hz: 12.0,
            am_depth: 1.0,
            rms: 0.9,
            reverb_mix: 0.8,
            reverb_decay: 0.9,
            ..SynthParams::default()
        };
        let pcm = synthesize(&p, SR);
        assert!(!pcm.is_empty());
        for &s in &pcm {
            assert!(s.is_finite(), "non-finite sample");
            assert!(s.abs() <= 1.0, "clip: {s}");
        }
    }

    #[test]
    fn rms_normalized_to_target() {
        let p = SynthParams {
            rms: 0.25,
            reverb_mix: 0.0,
            ..SynthParams::default()
        };
        let pcm = synthesize(&p, SR);
        let rms = (pcm.iter().map(|x| x * x).sum::<f32>() / pcm.len() as f32).sqrt();
        // Within a hair of the target (peak_guard only ever scales DOWN, and this
        // cue's peak after normalize stays below 0.99, so it is untouched).
        assert!((rms - 0.25).abs() < 0.02, "rms {rms} not near target 0.25");
    }

    #[test]
    fn envelope_starts_and_ends_near_silence() {
        let p = SynthParams {
            waveform: Waveform::Sine,
            reverb_mix: 0.0,
            adsr: Adsr { attack_ms: 10.0, decay_ms: 20.0, sustain: 0.6, release_ms: 40.0 },
            ..SynthParams::default()
        };
        let pcm = synthesize(&p, SR);
        // First sample is at the very start of the attack -> ~0; the very last is at
        // the end of the release -> ~0.
        assert!(pcm[0].abs() < 0.05, "attack should start near silence: {}", pcm[0]);
        assert!(pcm[pcm.len() - 1].abs() < 0.05, "release should end near silence");
    }

    #[test]
    fn degenerate_params_are_silent_not_poison() {
        let p = SynthParams {
            duration_ms: 0.0,
            ..SynthParams::default()
        };
        assert!(synthesize(&p, SR).is_empty());
        // A zero sample-rate must not divide-by-zero or NaN.
        let p2 = SynthParams::default();
        let pcm = synthesize(&p2, 0);
        for &s in &pcm {
            assert!(s.is_finite());
        }
    }

    #[test]
    fn wav16_has_a_valid_header() {
        let pcm = synthesize(&SynthParams { duration_ms: 50.0, ..SynthParams::default() }, SR);
        let wav = to_wav16(&pcm, SR);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // 44-byte header + 2 bytes/sample.
        assert_eq!(wav.len(), 44 + pcm.len() * 2);
        // The RIFF size field is the file length minus 8.
        let riff_len = u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]);
        assert_eq!(riff_len as usize, wav.len() - 8);
    }

    #[test]
    fn params_parse_from_sparse_toml() {
        // A theme declares only a couple of tokens; the rest default.
        let p: SynthParams = toml::from_str("waveform = \"triangle\"\nfreq_hz = 880.0\n").unwrap();
        assert_eq!(p.waveform, Waveform::Triangle);
        assert_eq!(p.freq_hz, 880.0);
        assert_eq!(p.duration_ms, default_duration_ms());
    }
}
