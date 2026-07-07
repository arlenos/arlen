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

/// Open the picker for a request + load the sources. Live: driven by the portal
/// event + `list_capture_sources`; fixture under vite.
export async function openSourcePicker(): Promise<void> {
  current.set(FIXTURE_REQUEST);
  try {
    sources.set(await invoke<Sources>("list_capture_sources"));
  } catch {
    sources.set(FIXTURE_SOURCES);
  }
}

/// Share the picked source. Live: `start_screencast` binds the source + cursor +
/// persist and returns the PipeWire stream.
export async function share(choice: ShareChoice): Promise<void> {
  current.set(null);
  try {
    await invoke("start_screencast", { ...choice });
  } catch {
    // No portal under vite: the optimistic close stands.
  }
}

/// Decline the request (deny is first-class). Live: resolve the portal request as
/// cancelled.
export async function cancel(): Promise<void> {
  current.set(null);
  try {
    await invoke("cancel_screencast");
  } catch {
    // mock
  }
}
