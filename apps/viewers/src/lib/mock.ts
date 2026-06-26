/// Mock data for the in-window surfaces while the decode backend (the Rust
/// `decode-*` workers) is wired separately by the coder. The shapes mirror
/// what a worker will hand the frontend: an audio probe plus the waveform
/// peaks the player renders. Replaced by real `invoke` results once the
/// host bridges the workers; nothing here ships in the product.

/// One audio file's player state (probe + computed peaks + playback clock).
export interface AudioMock {
  /// Tag title, falling back to the file stem when untagged.
  title: string;
  /// Tag artist, or null when the file carries no artist tag.
  artist: string | null;
  /// Codec short name (matches `AudioInfo.codec`).
  codec: string;
  /// Total duration in seconds.
  durationSec: number;
  /// Normalised 0..1 amplitude peaks, one per waveform bar.
  peaks: number[];
  /// Position in the folder, for the transient `n / total`.
  index: number;
  /// Folder size.
  total: number;
}

/// A deterministic speech/music-like peak envelope, so the mock waveform looks
/// plausible and is stable across renders (no `Math.random`, which would make
/// every screenshot differ).
export function mockPeaks(count = 180, seed = 7): number[] {
  let s = seed;
  const rnd = () => (s = (s * 9301 + 49297) % 233280) / 233280;
  const out: number[] = [];
  for (let i = 0; i < count; i++) {
    const t = i / count;
    const env = 0.22 + 0.78 * Math.abs(Math.sin(t * 30) * 0.5 + Math.sin(t * 9) * 0.5);
    out.push(Math.max(0.04, env * (0.5 + rnd() * 0.5)));
  }
  return out;
}

/// The audio fixture the demo route renders.
export const audioMock: AudioMock = {
  title: "Nightswim",
  artist: "Unknown artist",
  codec: "FLAC",
  durationSec: 220,
  peaks: mockPeaks(),
  index: 3,
  total: 18,
};
