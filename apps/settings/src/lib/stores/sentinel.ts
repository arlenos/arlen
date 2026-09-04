/// The Physical-World Privacy Sentinel state (privacy-sentinel-plan.md): five
/// detectors, deterministic ones default-on, ambient watchers opt-in. The
/// `org.arlen.Sentinel1` daemon and its Tauri bridge are coder seams
/// (`sentinel_get_state`, `sentinel_set_detector`, `sentinel_set_alerts`,
/// `sentinel_set_sensitivity`); under vite a fixture stands in so every card
/// state renders and drives.
import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

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

/// What was measured on one surface.
export type PostureName = "exposed" | "protected" | "unknown";

/// One line of the exposure posture readout.
///
/// It names a SURFACE and what was found there, never a sentence. The daemon
/// cannot write the prose: every string a person reads comes from this app's own
/// catalogue, so a daemon returning English would be a line no locale can reach.
/// The page turns the pair into a sentence.
export interface PostureLine {
  surface: string;
  posture: PostureName;
  /// Whether the daemon offers a one-click fix for it as it stands.
  fix?: boolean;
}

/// The whole sentinel state.
export interface SentinelState {
  detectors: Record<DetectorId, DetectorState>;
  posture: PostureLine[];
  /// Whether anything is using the microphone or camera right now, or absent
  /// when nothing could answer.
  ///
  /// The distinction is the sharpest one on this page. There is no microphone or
  /// camera portal in this build, so nothing can say whether something is
  /// capturing; "Nothing is using the microphone" on no evidence is the sentence
  /// this surface exists to avoid, and an absent value renders as not measured.
  captureActive?: boolean | null;
  /// The tracker needs a location grant; false = the card shows the re-grant line.
  trackerHasLocation: boolean;
  /// Set when a surface could not be read, so the readout is partial and says so.
  postureIncomplete?: boolean;
}

const FIXTURE: SentinelState = {
  detectors: {
    exposure: { on: true, alerts: "quiet" },
    usb: { on: true, alerts: "notify" },
    recording: { on: false, alerts: "quiet", sensitivity: "room" },
    tracker: { on: false, alerts: "notify", sensitivity: "balanced" },
  },
  posture: [
    { surface: "bluetooth_discoverable", posture: "exposed", fix: true },
    { surface: "ble_privacy", posture: "unknown" },
    { surface: "wifi_mac", posture: "protected" },
    { surface: "hidden_network", posture: "protected" },
  ],
  postureIncomplete: true,
  captureActive: null,
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
    if (!tauriAvailable) {
      sentinel.set(structuredClone(FIXTURE));
      sentinelMocked.set(true);
      sentinelUnavailable.set(false);
      return;
    }
    // A real session with nothing to read. Worth being exact about which:
    // `arlen-sentineld` now answers this, so this branch is what it was always
    // worded for: the daemon is not running, or it is and the ask failed. Either
    // way nothing measured this machine, and "nothing is reporting" is the
    // honest thing to say. It was written while there was no daemon at all and
    // the wording did not have to change when one landed.
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
    if (!tauriAvailable) return; // no host to write through
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
    if (!tauriAvailable) return; // no host to write through
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
    if (!tauriAvailable) return; // no host to write through
    if (before !== undefined) update(id, { sensitivity: before });
    sentinelChangeFailed.set(true);
  }
}

/// Run the one-click remediation behind a posture line (e.g. stop Bluetooth
/// being discoverable). Live: the daemon applies it and re-reads.
///
/// Addressed by SURFACE, not by the line's position. The readout is recomputed on
/// every read and sorted worst-first, so a radio that changed between the read
/// and the tap moves the lines and an index would point the fix at the
/// neighbouring one.
export async function fixPosture(surface: string): Promise<void> {
  sentinelChangeFailed.set(false);
  try {
    await invoke("sentinel_fix_posture", { surface });
    await loadSentinel();
  } catch {
    // The fixture stays under the DEV gate. Outside it, this catch used to write
    // "Bluetooth is no longer discoverable." into the posture line whatever the
    // command did - and `sentinel_fix_posture` has no daemon behind it, so in a
    // real session the fix ALWAYS failed and the page ALWAYS said it had worked.
    // A protection page claiming a machine is secured when nothing was done is
    // the one lie on this surface that costs more than showing nothing, which is
    // what the flag below already says about the detector switches.
    if (tauriAvailable) {
      sentinelChangeFailed.set(true);
      return;
    }
    sentinel.update((s) => {
      if (!s) return s;
      const posture = s.posture.map((p) =>
        p.surface === surface ? { surface, posture: "protected" as const } : p,
      );
      return { ...s, posture };
    });
  }
}
