/// The Physical-World Privacy Sentinel state (privacy-sentinel-plan.md): five
/// detectors, deterministic ones default-on, ambient watchers opt-in. The
/// `org.arlen.Sentinel1` daemon and its Tauri bridge are coder seams
/// (`sentinel_get_state`, `sentinel_set_detector`, `sentinel_set_alerts`,
/// `sentinel_set_sensitivity`); under vite a fixture stands in so every card
/// state renders and drives.
import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The detectors the surface configures (mic/cam is status-only, owned by the
/// capture infrastructure).
export type DetectorId = "exposure" | "usb" | "recording" | "tracker";
/// How a detector speaks when it finds something.
export type AlertMode = "quiet" | "notify";

/// One detector's configuration.
export interface DetectorState {
  on: boolean;
  alerts: AlertMode;
  /// Present only where sensitivity means something (recording: proximity,
  /// tracker: strictness).
  sensitivity?: string;
}

/// One line of the exposure posture readout; `fix` marks the one-click
/// remediation the daemon offers for it.
export interface PostureLine {
  text: string;
  fix?: boolean;
}

/// The whole sentinel state.
export interface SentinelState {
  detectors: Record<DetectorId, DetectorState>;
  posture: PostureLine[];
  /// Whether anything is using the microphone or camera right now (the
  /// capture-infra signal the sentinel subscribes to).
  captureActive: boolean;
  /// The tracker needs a location grant; false = the card shows the re-grant line.
  trackerHasLocation: boolean;
}

const FIXTURE: SentinelState = {
  detectors: {
    exposure: { on: true, alerts: "quiet" },
    usb: { on: true, alerts: "notify" },
    recording: { on: false, alerts: "quiet", sensitivity: "room" },
    tracker: { on: false, alerts: "notify", sensitivity: "balanced" },
  },
  posture: [
    { text: "Wi-Fi uses a different hardware address for every network." },
    { text: "Saved networks are not broadcast while disconnected." },
    { text: "Bluetooth is discoverable right now.", fix: true },
  ],
  captureActive: false,
  trackerHasLocation: false,
};

/// The sentinel state, or null until the first read settles.
export const sentinel = writable<SentinelState | null>(null);
/// True while the state is the FIXTURE rather than this machine's real posture.
export const sentinelMocked = writable(false);

/// True when a real session could not read the state at all.
///
/// Distinct from `sentinelMocked`. The page guards its whole body on `$sentinel`
/// being set, so a null read renders an empty page - and an empty privacy page
/// reads as "nothing to report", which is the one thing it must not say when it
/// does not know.
export const sentinelUnavailable = writable(false);

/// True when the last change to a detector did not reach the service, so the
/// switch went back. A protection page that shows a detector as on when it is
/// off is worse than one that shows nothing: somebody stops looking.
export const sentinelChangeFailed = writable(false);

/// Load the state. Live: `sentinel_get_state`; fixture under vite.
export async function loadSentinel(): Promise<void> {
  try {
    sentinel.set(await invoke<SentinelState>("sentinel_get_state"));
    sentinelMocked.set(false);
  } catch {
    if (import.meta.env.DEV) {
      sentinel.set(structuredClone(FIXTURE));
      sentinelMocked.set(true);
      sentinelUnavailable.set(false);
      return;
    }
    // A real session with nothing to read. Worth being exact about which:
    // `sentinel_get_state` has NO backend anywhere - no Tauri command, and no
    // daemon owns `org.arlen.Sentinel1`; only this store and the pure detector
    // crate's doc comment mention the name. So this branch is not a transient
    // failure, it is the permanent state of this build, and the wording says
    // "nothing is reporting" rather than "cannot read right now", which would
    // invite a retry that cannot succeed. It stays true if the daemon lands and
    // is merely down.
    //
    // A real session that could not read the sentinel. The fixture asserts that
    // the Wi-Fi address rotates, that saved networks are not broadcast and -
    // the sharpest one - that nothing is using the microphone or camera right
    // now. That last line is what a person opens this page to find out, and
    // "example state" above it does not unsay it. Null, and the page says it
    // cannot read the posture.
    sentinel.set(null);
    sentinelMocked.set(false);
    sentinelUnavailable.set(true);
  }
}

function update(id: DetectorId, patch: Partial<DetectorState>): void {
  sentinel.update((s) => {
    if (!s) return s;
    return { ...s, detectors: { ...s.detectors, [id]: { ...s.detectors[id], ...patch } } };
  });
}

/// Turn a detector on or off. The always-on pair's OFF path goes through the
/// acknowledge dialog in the page before this is called.
export async function setDetector(id: DetectorId, on: boolean): Promise<void> {
  const before = get(sentinel)?.detectors[id]?.on;
  update(id, { on });
  sentinelChangeFailed.set(false);
  try {
    await invoke("sentinel_set_detector", { id, on });
  } catch {
    if (import.meta.env.DEV) return; // seam unwired under vite
    if (before !== undefined) update(id, { on: before });
    sentinelChangeFailed.set(true);
  }
}

/// Switch a detector between staying quiet and notifying.
export async function setAlerts(id: DetectorId, alerts: AlertMode): Promise<void> {
  const before = get(sentinel)?.detectors[id]?.alerts;
  update(id, { alerts });
  sentinelChangeFailed.set(false);
  try {
    await invoke("sentinel_set_alerts", { id, mode: alerts });
  } catch {
    if (import.meta.env.DEV) return; // seam unwired under vite
    if (before !== undefined) update(id, { alerts: before });
    sentinelChangeFailed.set(true);
  }
}

/// Set a watcher's sensitivity (proximity for the recording indicator, the
/// strictness bar for the tracker).
export async function setSensitivity(id: DetectorId, level: string): Promise<void> {
  const before = get(sentinel)?.detectors[id]?.sensitivity;
  update(id, { sensitivity: level });
  sentinelChangeFailed.set(false);
  try {
    await invoke("sentinel_set_sensitivity", { id, level });
  } catch {
    if (import.meta.env.DEV) return; // seam unwired under vite
    if (before !== undefined) update(id, { sensitivity: before });
    sentinelChangeFailed.set(true);
  }
}

/// Run the one-click remediation behind a posture line (e.g. stop Bluetooth
/// being discoverable). Live: the daemon applies it and re-reads.
export async function fixPosture(index: number): Promise<void> {
  sentinelChangeFailed.set(false);
  try {
    await invoke("sentinel_fix_posture", { index });
    await loadSentinel();
  } catch {
    // The fixture stays under the DEV gate. Outside it, this catch used to write
    // "Bluetooth is no longer discoverable." into the posture line whatever the
    // command did - and `sentinel_fix_posture` has no daemon behind it, so in a
    // real session the fix ALWAYS failed and the page ALWAYS said it had worked.
    // A protection page claiming a machine is secured when nothing was done is
    // the one lie on this surface that costs more than showing nothing, which is
    // what the flag below already says about the detector switches.
    if (!import.meta.env.DEV) {
      sentinelChangeFailed.set(true);
      return;
    }
    sentinel.update((s) => {
      if (!s) return s;
      const posture = s.posture.map((p, i) =>
        i === index ? { text: "Bluetooth is no longer discoverable." } : p,
      );
      return { ...s, posture };
    });
  }
}
