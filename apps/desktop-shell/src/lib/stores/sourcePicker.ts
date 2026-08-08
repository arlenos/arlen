/// The screencast source-picker (screenshot-capture-plan.md §3): the "what do I
/// share" chooser shown when an app requests a screencast (the portal ScreenCast
/// SelectSources -> Start negotiation). It is a consent moment - an app wants to
/// capture your screen - so it carries the consent framing (who is asking, deny
/// first-class, only what you pick is sent).
///
/// Mock-vs-live: fixture-backed. The portal ScreenCast backend (CreateSession ->
/// SelectSources -> Start, the PipeWire stream, the restore_token/persist wiring),
/// `list_capture_sources` (live monitors + windows), `start_screencast`, and the
/// portal-event -> `current` feed are coder seams; under vite the store serves a
/// fixture so the surface renders.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The app asking to capture, from the portal request.
export interface SourceRequest {
  requester: string;
  requesterLabel: string;
  /// SelectSources `multiple` - whether more than one source may be picked.
  multiple: boolean;
}

/// A monitor (portal MONITOR source).
export interface Monitor {
  id: string;
  name: string;
  resolution: string;
}

/// A window (portal WINDOW source, from ext-foreign-toplevel-list-v1).
export interface Win {
  id: string;
  appLabel: string;
  title: string;
}

export interface Sources {
  monitors: Monitor[];
  windows: Win[];
}

/// What the picker returns: the picked source + the cursor/persist choices.
export interface ShareChoice {
  kind: "monitor" | "window" | "region";
  id: string;
  /// cursor_mode: embedded (show) vs hidden.
  showCursor: boolean;
  /// persist_mode: remember (until-revoked) vs none (ask each time).
  remember: boolean;
}

const FIXTURE_REQUEST: SourceRequest = {
  requester: "com.example.meet",
  requesterLabel: "Meet",
  multiple: false,
};
const FIXTURE_SOURCES: Sources = {
  monitors: [
    { id: "m1", name: "Built-in display", resolution: "2560 x 1600" },
    { id: "m2", name: "Dell U2720Q", resolution: "3840 x 2160" },
  ],
  windows: [
    { id: "w1", appLabel: "Firefox", title: "Arlen OS - Wikipedia" },
    { id: "w2", appLabel: "Terminal", title: "tim@arlen: ~/work" },
    { id: "w3", appLabel: "Files", title: "Documents" },
  ],
};

/// The active request, or null when nothing is being asked.
export const current = writable<SourceRequest | null>(null);
/// The sources to choose from.
export const sources = writable<Sources>({ monitors: [], windows: [] });

/// True while the source list is the FIXTURE (named displays, real-looking window
/// titles) rather than this machine's actual capturable surfaces. Picking one to
/// share is a privacy decision, so the list must not pass as real.
export const sourcesMocked = writable(false);

/// True when the source list could not be read in a real session.
///
/// Distinct from an empty list, which would mean this machine has nothing to
/// capture - a sentence that is almost never true and that hides the actual
/// problem. And distinct from `sourcesMocked`, which is design work under vite.
export const sourcesUnavailable = writable(false);

/// True when the last share attempt did not reach the portal, so nothing is
/// being shared and the picker is still open. Closing on failure would tell
/// someone their screen is going out when it is not, and they would carry on
/// as if the other side could see it.
export const shareFailed = writable(false);

/// Open the picker for a request + load the sources. Live: driven by the portal
/// event + `list_capture_sources`; fixture sources under vite.
///
/// `request` is REQUIRED and carries the real requesting app. It used to be
/// unconditionally `FIXTURE_REQUEST` (`com.example.meet`, "Meet") with no way to
/// pass the caller in - so once the portal wired this up, the consent dialog
/// would have named "Meet" no matter which app actually asked. In a screen-capture
/// prompt the requester IS the fact the user decides on, so a wrong name there
/// grants capture to the wrong app. There are no callers yet; the portal wiring
/// must supply the attested requester, and the demo helper below passes the
/// fixture EXPLICITLY so the live path can never fall back to it.
export async function openSourcePicker(request: SourceRequest): Promise<void> {
  current.set(request);
  try {
    sources.set(await invoke<Sources>("list_capture_sources"));
    sourcesMocked.set(false);
    sourcesUnavailable.set(false);
  } catch {
    if (import.meta.env.DEV) {
      // No backend under vite: the fixture is the honest thing to render for
      // design work, and `sourcesMocked` labels it.
      sources.set(FIXTURE_SOURCES);
      sourcesMocked.set(true);
      sourcesUnavailable.set(false);
      return;
    }
    // A real session. The paragraph above says the demo helper passes the fixture
    // EXPLICITLY "so the live path can never fall back to it" - and this catch
    // was that fallback, doing it unconditionally. Offering "Firefox - Arlen OS -
    // Wikipedia" when we could not read the real windows is worse than offering
    // nothing: the user shares a surface believing they picked it.
    sources.set({ monitors: [], windows: [] });
    sourcesMocked.set(false);
    sourcesUnavailable.set(true);
  }
}

/// Open the picker against the fixture requester - dev/demo only, never a live
/// portal path (which must pass its own attested `SourceRequest`).
export async function openSourcePickerDemo(): Promise<void> {
  await openSourcePicker(FIXTURE_REQUEST);
}

/// Share the picked source. Live: `start_screencast` binds the source + cursor +
/// persist and returns the PipeWire stream.
export async function share(choice: ShareChoice): Promise<void> {
  shareFailed.set(false);
  try {
    await invoke("start_screencast", { ...choice });
  } catch {
    if (import.meta.env.DEV) {
      current.set(null); // no portal under vite: the flow stays drivable
      return;
    }
    // The picker stays open: the decision has not been carried out, so the
    // interaction is not over.
    shareFailed.set(true);
    return;
  }
  current.set(null);
}

/// Decline the request (deny is first-class). Live: resolve the portal request as
/// cancelled.
export async function cancel(): Promise<void> {
  // Declining closes the picker either way. A cancel that did not reach the
  // portal leaves the request pending there, which is the safe direction -
  // nothing is shared - and holding a dialog open over a refusal the user has
  // already made would be its own kind of dishonesty.
  current.set(null);
  shareFailed.set(false);
  try {
    await invoke("cancel_screencast");
  } catch {
    // Nothing to correct on screen: no sharing started, and the picker is closed
    // because the user declined. Written as a statement so the check sees the
    // decision instead of an empty block.
    shareFailed.set(false);
  }
}
