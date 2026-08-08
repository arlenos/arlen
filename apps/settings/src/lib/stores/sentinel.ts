/// The Physical-World Privacy Sentinel state (privacy-sentinel-plan.md): five
/// detectors, deterministic ones default-on, ambient watchers opt-in. The
/// `org.arlen.Sentinel1` daemon and its Tauri bridge are coder seams
/// (`sentinel_get_state`, `sentinel_set_detector`, `sentinel_set_alerts`,
/// `sentinel_set_sensitivity`); under vite a fixture stands in so every card
/// state renders and drives.
import { writable } from "svelte/store";
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
  update(id, { on });
  try {
    await invoke("sentinel_set_detector", { id, on });
  } catch {
    // Seam unwired: the local view stands.
  }
}

/// Switch a detector between staying quiet and notifying.
export async function setAlerts(id: DetectorId, alerts: AlertMode): Promise<void> {
  update(id, { alerts });
  try {
    await invoke("sentinel_set_alerts", { id, mode: alerts });
  } catch {
    // Seam unwired.
  }
}

/// Set a watcher's sensitivity (proximity for the recording indicator, the
/// strictness bar for the tracker).
export async function setSensitivity(id: DetectorId, level: string): Promise<void> {
  update(id, { sensitivity: level });
  try {
    await invoke("sentinel_set_sensitivity", { id, level });
  } catch {
    // Seam unwired.
  }
}

/// Run the one-click remediation behind a posture line (e.g. stop Bluetooth
/// being discoverable). Live: the daemon applies it and re-reads.
export async function fixPosture(index: number): Promise<void> {
  try {
    await invoke("sentinel_fix_posture", { index });
    await loadSentinel();
  } catch {
    // Fixture: apply the remediation locally so the flow is drivable.
    sentinel.update((s) => {
      if (!s) return s;
      const posture = s.posture.map((p, i) =>
        i === index ? { text: "Bluetooth is no longer discoverable." } : p,
      );
      return { ...s, posture };
    });
  }
}
