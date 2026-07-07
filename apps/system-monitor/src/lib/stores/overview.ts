/// The system monitor's Overview tab (system-monitor-plan.md): the glanceable
/// default - a calm verdict, what's using sensitive resources right now, and what's
/// using the most. The monitor is a READ-ONLY lens over the event bus + audit
/// ledger; it owns time, never grant state. Every live-access row deep-links to the
/// App-access page, which holds the revoke.
///
/// Mock-vs-live: fixture-backed. `system_verdict` / `live_access` / `resource_top`
/// (daemon-health roll-up + portal/compositor sensor use + eBPF) are coder seams on
/// the deferred monitor backend; under vite the store serves a fixture.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// The overall health/attention state, one of three (drives the single status accent).
export type VerdictState = "normal" | "attention" | "alert";

/// The glanceable headline verdict.
export interface Verdict {
  state: VerdictState;
  headline: string;
  detail: string;
}

/// One app using a sensitive resource right now.
export interface LiveAccess {
  app: string;
  appId: string;
  /// camera / microphone / screen / network / location.
  resources: string[];
  sinceMins: number;
}

/// One app's resource use, for the "using the most" list.
export interface ResourceUse {
  app: string;
  appId: string;
  cpu: number;
  memMB: number;
}

interface OverviewState {
  verdict: Verdict;
  liveAccess: LiveAccess[];
  resourceTop: ResourceUse[];
  mocked: boolean;
}

const FIXTURE = {
  verdict: {
    state: "normal" as VerdictState,
    headline: "Everything's running normally.",
    detail: "No unusual access, and every Arlen service is healthy.",
  },
  liveAccess: [
    { app: "Meet", appId: "com.google.meet", resources: ["microphone", "camera"], sinceMins: 12 },
  ] as LiveAccess[],
  resourceTop: [
    { app: "Firefox", appId: "org.mozilla.firefox", cpu: 14, memMB: 1840 },
    { app: "Meet", appId: "com.google.meet", cpu: 9, memMB: 920 },
    { app: "Knowledge graph", appId: "org.arlen.knowledge", cpu: 2, memMB: 340 },
    { app: "Files", appId: "org.arlen.files", cpu: 1, memMB: 210 },
  ] as ResourceUse[],
};

export const overview = writable<OverviewState>({ ...FIXTURE, mocked: false });

/// Load the overview. Live: the three commands; fixture under vite.
export async function load(): Promise<void> {
  try {
    const [verdict, liveAccess, resourceTop] = await Promise.all([
      invoke<Verdict>("system_verdict"),
      invoke<LiveAccess[]>("live_access"),
      invoke<ResourceUse[]>("resource_top"),
    ]);
    overview.set({ verdict, liveAccess, resourceTop, mocked: false });
  } catch {
    overview.set({ ...FIXTURE, mocked: true });
  }
}

/// Open the App-access page for a principal (where the revoke lives). Live seam:
/// a cross-app launch of Settings at the privacy page, filtered to the app.
export async function manageAccess(appId: string): Promise<void> {
  try {
    await invoke("open_app_access", { appId });
  } catch {
    // No cross-app launch under vite: the monitor never holds the revoke itself.
  }
}
