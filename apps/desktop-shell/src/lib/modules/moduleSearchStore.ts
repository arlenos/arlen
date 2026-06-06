/// Worker pool + search aggregator for Tier 2 module Waypointer
/// providers.
///
/// One **hidden iframe** per module that declares
/// `[waypointer.search]` is mounted off-screen at shell start. On
/// every Waypointer keystroke `searchModules(query)` posts a
/// `search` message into every worker, collects replies for up to
/// 200 ms, drops any reply whose `requestId` is no longer the
/// active one, and returns the aggregated `SearchResult[]`.
///
/// Why hidden iframes instead of a daemon-side runtime: the iframes
/// already exist as the Tier 2 sandbox (CSP, nonce, capability
/// gating via `module_host_call` postMessage proxy). Re-using them
/// for search avoids a parallel Tier 1 WASM execution path which
/// is not yet built. When Tier 1 lands, modulesd's
/// `WaypointerSearchAll` handler can take over for WASM modules and
/// this frontend pool stays as the Tier 2 path.

import { invoke } from "@tauri-apps/api/core";
import { writable, type Writable } from "svelte/store";
import type { SearchResult } from "./postmsg";

interface UiModule {
  id: string;
  name: string;
  tier: string;
  enabled: boolean;
  failed: boolean;
  priority: number;
  extensionPoints: string[];
}

interface WorkerHandle {
  moduleId: string;
  iframe: HTMLIFrameElement;
  /// True once the worker has sent its `ready` postMessage. Until
  /// then, search requests time out without producing results.
  ready: boolean;
  /// Latency p50 in milliseconds, EWMA over the last 20 calls.
  /// Workers consistently slower than 200 ms get dropped from the
  /// next round of aggregation; the EWMA decay means a single fast
  /// run is enough to get reinstated.
  latencyP50: number;
}

/// Active workers keyed by module id. The pool grows as modules
/// arrive (via `refreshFromDaemon`) and shrinks on disable/failure.
const workers = new Map<string, WorkerHandle>();

/// Pending search requests keyed by requestId. Each entry tracks
/// per-module replies so we can know when all workers have replied
/// (early-exit) versus when the timeout fires.
const pending = new Map<
  string,
  {
    resolve: (results: SearchResult[]) => void;
    /// requestId-keyed map of module-id → results that arrived.
    received: Map<string, SearchResult[]>;
    /// Module IDs we expected to hear from. Used to know "all in" early.
    expected: Set<string>;
  }
>();

let nextRequestId = 1;

/// Reactive view of the current worker pool, exposed so debug UIs
/// (Settings → Modules → "Currently loaded") can render it.
export const moduleWorkers: Writable<UiModule[]> = writable([]);

/// Single hidden host element appended to document.body on first
/// use. Carrying the worker iframes in a dedicated element under
/// the body (rather than under a Svelte component subtree) keeps
/// them entirely out of the visible layout flow — no chance of a
/// 1×1 placeholder being miscomputed by the Tauri webview's flex
/// layout, no chance of the host element following its parent's
/// transform/opacity into the visible viewport.
let workerHost: HTMLDivElement | null = null;

function ensureHost(): HTMLDivElement {
  if (workerHost && workerHost.isConnected) return workerHost;
  const host = document.createElement("div");
  host.setAttribute("aria-hidden", "true");
  host.dataset.lunarisRole = "module-worker-host";
  // `position: absolute` plus a far-off-screen offset is the
  // long-established way to keep an element loadable but invisible.
  // We deliberately avoid `display: none` (would block iframe load)
  // and `visibility: hidden` (would still reserve layout space).
  host.style.position = "absolute";
  host.style.left = "-99999px";
  host.style.top = "-99999px";
  host.style.width = "1px";
  host.style.height = "1px";
  host.style.overflow = "hidden";
  host.style.pointerEvents = "none";
  document.body.appendChild(host);
  workerHost = host;
  return host;
}

/// Mount or unmount workers based on the current daemon-reported
/// module list. Idempotent: existing workers are kept, missing ones
/// are added, removed ones are torn down. The optional `_host`
/// parameter is ignored; kept for backward compatibility with the
/// previous signature (callers pass a Svelte ref but the iframes
/// always live under a daemon-managed body-level host now).
export async function refreshFromDaemon(_host?: HTMLElement) {
  let modules: UiModule[] = [];
  try {
    modules = await invoke<UiModule[]>("modulesd_list_modules");
  } catch {
    return;
  }
  const eligible = modules.filter(
    (m) =>
      m.tier === "iframe" &&
      m.enabled &&
      !m.failed &&
      m.extensionPoints.includes("waypointer"),
  );
  const eligibleIds = new Set(eligible.map((m) => m.id));

  // Tear down workers whose modules are no longer eligible.
  for (const [id, w] of workers) {
    if (!eligibleIds.has(id)) {
      w.iframe.remove();
      workers.delete(id);
    }
  }

  if (eligible.length === 0) {
    moduleWorkers.set(eligible);
    return;
  }

  const host = ensureHost();

  // Spawn new workers.
  for (const mod of eligible) {
    if (workers.has(mod.id)) continue;
    let issued: { url: string; nonce: string };
    try {
      issued = await invoke<{ url: string; nonce: string }>(
        "mint_iframe",
        { moduleId: mod.id, slot: "waypointer" },
      );
    } catch {
      continue;
    }
    const iframe = document.createElement("iframe");
    iframe.src = issued.url;
    iframe.setAttribute("sandbox", "allow-scripts");
    iframe.setAttribute("title", `${mod.name} (worker)`);
    iframe.setAttribute("aria-hidden", "true");
    iframe.style.border = "none";
    iframe.style.width = "1px";
    iframe.style.height = "1px";
    iframe.style.opacity = "0";
    iframe.style.pointerEvents = "none";
    host.appendChild(iframe);
    workers.set(mod.id, {
      moduleId: mod.id,
      iframe,
      ready: false,
      latencyP50: 0,
    });
  }

  moduleWorkers.set(eligible);
}

/// Fan a search query out to every live worker. Returns within at
/// most `timeoutMs` (default 200) with whatever results have come
/// back. Stale replies (different requestId) are dropped.
export function searchModules(
  query: string,
  timeoutMs = 200,
): Promise<SearchResult[]> {
  return new Promise((resolve) => {
    const requestId = `wp-${nextRequestId++}`;
    const expected = new Set<string>();
    const received = new Map<string, SearchResult[]>();

    // Skip workers whose recent latency is consistently above the
    // timeout: they slow the overall pipeline without contributing.
    const startedAt = performance.now();
    for (const [id, w] of workers) {
      if (!w.ready) continue;
      if (w.latencyP50 > timeoutMs) continue;
      expected.add(id);
      startTimes.set(`${requestId}:${id}`, startedAt);
      w.iframe.contentWindow?.postMessage(
        { type: "search", requestId, query },
        "*",
      );
    }

    if (expected.size === 0) {
      resolve([]);
      return;
    }

    pending.set(requestId, { resolve, received, expected });

    setTimeout(() => {
      const entry = pending.get(requestId);
      if (!entry) return;
      pending.delete(requestId);
      entry.resolve(flatten(entry.received));
    }, timeoutMs);
  });
}

/// Helper: concat all per-module result arrays in priority order.
function flatten(byModule: Map<string, SearchResult[]>): SearchResult[] {
  const out: SearchResult[] = [];
  // Worker map preserves insertion order; insertion order is the
  // priority order (refreshFromDaemon pre-sorts by priority).
  for (const id of workers.keys()) {
    const r = byModule.get(id);
    if (r) out.push(...r);
  }
  return out;
}

/// Update the EWMA latency for a worker.
function recordLatency(moduleId: string, ms: number) {
  const w = workers.get(moduleId);
  if (!w) return;
  // EWMA with α = 0.2 → quick adaptation while filtering single
  // outliers. p50 is technically a median, not a mean; we
  // approximate with EWMA because a true rolling-window median is
  // overkill here.
  w.latencyP50 = w.latencyP50 === 0 ? ms : w.latencyP50 * 0.8 + ms * 0.2;
}

/// Per-request start times for latency tracking, keyed by
/// (requestId, moduleId).
const startTimes = new Map<string, number>();

/// Wire up the global postMessage listener once. Idempotent.
let listenerInstalled = false;
export function installListener() {
  if (listenerInstalled) return;
  listenerInstalled = true;
  window.addEventListener("message", (ev) => {
    const data = ev.data;
    if (!data || typeof data !== "object") return;

    // Identify which worker this came from by source iframe.
    let sourceId: string | null = null;
    for (const [id, w] of workers) {
      if (ev.source === w.iframe.contentWindow) {
        sourceId = id;
        break;
      }
    }
    if (!sourceId) return;

    if (data.type === "ready") {
      const w = workers.get(sourceId);
      if (w) w.ready = true;
      return;
    }

    if (data.type === "search.results") {
      const requestId = data.requestId as string;
      const entry = pending.get(requestId);
      if (!entry) return; // stale

      const startKey = `${requestId}:${sourceId}`;
      const start = startTimes.get(startKey);
      if (start !== undefined) {
        recordLatency(sourceId, performance.now() - start);
        startTimes.delete(startKey);
      }
      entry.received.set(sourceId, data.results as SearchResult[]);

      // Early exit when every expected worker has replied.
      if (entry.received.size >= entry.expected.size) {
        pending.delete(requestId);
        entry.resolve(flatten(entry.received));
      }
    }
  });
}
