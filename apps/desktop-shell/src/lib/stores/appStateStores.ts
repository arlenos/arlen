/**
 * Per-app state stores derived from `app.shortcut.*`,
 * `app.badge.*`, `app.ambient.*` Event Bus events forwarded by
 * the Rust backend.
 *
 * All three are **per-app** (not per-window) — different from
 * the toolbar store which is per-(app, window). Multi-window
 * apps share one shortcut list / badge / ambient effect across
 * all their windows.
 *
 * Render: only the focused window's app's state shows. Other
 * apps' state stays in the store invisibly until they regain
 * focus.
 *
 * Keyed by the permission id, which is what these events are
 * published under. The focused window announces a different
 * name for the same app, so the focus side is resolved first -
 * see `activeApp.ts`. Keying on the window's own id, as this
 * did, left all three surfaces permanently empty.
 *
 * See `docs/architecture/{shortcuts,badges,ambient}-api.md`.
 */

import { derived, get, writable, type Readable } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { activeAppId } from "./activeApp";

// ── Shortcuts ───────────────────────────────────────────────────

export interface ShortcutEntry {
  label: string;
  icon: string;
  action: string;
  context: string[];
  confirm: string | null;
  /** Diff-update fields (initial register sets enabled=true, badge=null). */
  enabled: boolean;
  badge: string | null;
}

interface ShortcutsByApp {
  byApp: Map<string, ShortcutEntry[]>;
}

const shortcutsInternal = writable<ShortcutsByApp>({ byApp: new Map() });

interface ShortcutRegisterEvent {
  appId: string;
  shortcuts: Array<{
    label: string;
    icon: string;
    action: string;
    context: string[];
    confirm?: string | null;
  }>;
}
interface ShortcutStateEvent {
  appId: string;
  action: string;
  enabled?: boolean | null;
  badge?: string | null;
}
interface ShortcutClearedEvent {
  appId: string;
}

function applyShortcutRegister(e: ShortcutRegisterEvent) {
  shortcutsInternal.update((s) => {
    const next = new Map(s.byApp);
    if (e.shortcuts.length === 0) {
      next.delete(e.appId);
    } else {
      next.set(
        e.appId,
        e.shortcuts.map((sh) => ({
          label: sh.label,
          icon: sh.icon,
          action: sh.action,
          context: sh.context ?? [],
          confirm: sh.confirm ?? null,
          enabled: true,
          badge: null,
        })),
      );
    }
    return { byApp: next };
  });
}

function applyShortcutStateChanged(e: ShortcutStateEvent) {
  shortcutsInternal.update((s) => {
    const list = s.byApp.get(e.appId);
    if (!list) return s; // unknown app — silent no-op
    const next = new Map(s.byApp);
    next.set(
      e.appId,
      list.map((sh) =>
        sh.action === e.action
          ? {
              ...sh,
              enabled: e.enabled !== null && e.enabled !== undefined ? e.enabled : sh.enabled,
              badge:
                e.badge === undefined || e.badge === null
                  ? sh.badge
                  : e.badge === ""
                    ? null
                    : e.badge,
            }
          : sh,
      ),
    );
    return { byApp: next };
  });
}

function applyShortcutCleared(e: ShortcutClearedEvent) {
  shortcutsInternal.update((s) => {
    const next = new Map(s.byApp);
    next.delete(e.appId);
    return { byApp: next };
  });
}

/**
 * Shortcut list for the focused app, or [] if no app focused
 * or no registered shortcuts.
 */
export const focusedShortcuts: Readable<ShortcutEntry[]> = derived(
  [shortcutsInternal, activeAppId],
  ([$internal, $appId]) => {
    const appId = $appId;
    if (!appId) return [] as ShortcutEntry[];
    return $internal.byApp.get(appId) ?? [];
  },
);

// ── Badges ──────────────────────────────────────────────────────
//
// Variant integers come from the Rust enum:
//   0 = unspecified, 1 = count, 2 = dot, 3 = status,
//   4 = countWithStatus
//
// Status integers:
//   0 = unspecified, 1 = success, 2 = warning, 3 = error,
//   4 = progress

export type BadgeStatusName = "success" | "warning" | "error" | "progress";
export type BadgeRender =
  | { kind: "count"; count: number }
  | { kind: "dot" }
  | { kind: "status"; status: BadgeStatusName; value: number | null }
  | { kind: "countWithStatus"; count: number; status: BadgeStatusName }
  | null;

interface BadgeSetEvent {
  appId: string;
  variant: number;
  count: number;
  status: number;
  progressValue?: number | null;
}
interface BadgeClearedEvent {
  appId: string;
}

function statusFromInt(n: number): BadgeStatusName | null {
  switch (n) {
    case 1:
      return "success";
    case 2:
      return "warning";
    case 3:
      return "error";
    case 4:
      return "progress";
    default:
      return null;
  }
}

function decodeBadge(e: BadgeSetEvent): BadgeRender {
  switch (e.variant) {
    case 1:
      return { kind: "count", count: e.count };
    case 2:
      return { kind: "dot" };
    case 3: {
      const status = statusFromInt(e.status);
      if (!status) return null;
      return { kind: "status", status, value: e.progressValue ?? null };
    }
    case 4: {
      const status = statusFromInt(e.status);
      if (!status) return null;
      return { kind: "countWithStatus", count: e.count, status };
    }
    default:
      return null;
  }
}

const badgesInternal = writable<{ byApp: Map<string, BadgeRender> }>({
  byApp: new Map(),
});

function applyBadgeSet(e: BadgeSetEvent) {
  badgesInternal.update((s) => {
    const decoded = decodeBadge(e);
    const next = new Map(s.byApp);
    if (decoded === null) {
      next.delete(e.appId);
    } else {
      next.set(e.appId, decoded);
    }
    return { byApp: next };
  });
}

function applyBadgeCleared(e: BadgeClearedEvent) {
  badgesInternal.update((s) => {
    const next = new Map(s.byApp);
    next.delete(e.appId);
    return { byApp: next };
  });
}

export const focusedBadge: Readable<BadgeRender> = derived(
  [badgesInternal, activeAppId],
  ([$internal, $appId]) => {
    const appId = $appId;
    if (!appId) return null;
    return $internal.byApp.get(appId) ?? null;
  },
);

// ── Ambient ─────────────────────────────────────────────────────

export type AmbientEffectName = "pulse" | "tint";
export type AmbientColorName = "accent" | "warning" | "error" | "success";
export type AmbientSpeedName = "slow" | "medium" | "fast";

export interface AmbientRender {
  effect: AmbientEffectName;
  color: AmbientColorName;
  intensity: number;
  speed: AmbientSpeedName;
}

interface AmbientSetEvent {
  appId: string;
  effect: number;
  color: number;
  intensity: number;
  speed: number;
  reason: string;
  autoClearMs: number;
}
interface AmbientClearedEvent {
  appId: string;
}

function effectFromInt(n: number): AmbientEffectName | null {
  return n === 1 ? "pulse" : n === 2 ? "tint" : null;
}
function colorFromInt(n: number): AmbientColorName | null {
  switch (n) {
    case 1:
      return "accent";
    case 2:
      return "warning";
    case 3:
      return "error";
    case 4:
      return "success";
    default:
      return null;
  }
}
function speedFromInt(n: number): AmbientSpeedName | null {
  switch (n) {
    case 1:
      return "slow";
    case 2:
      return "medium";
    case 3:
      return "fast";
    default:
      return null;
  }
}

interface AmbientSlot {
  render: AmbientRender;
  /** Unix ms timestamp when the effect should auto-clear, or null. */
  expiresAt: number | null;
  /**
   * When this effect was set. The pick below needs it: a `Map`
   * keeps insertion order, and re-setting an existing key keeps
   * its ORIGINAL position, so "the last one set" is not
   * "the last one in the map".
   */
  setAt: number;
}

const ambientInternal = writable<{ byApp: Map<string, AmbientSlot> }>({
  byApp: new Map(),
});

function applyAmbientSet(e: AmbientSetEvent) {
  const effect = effectFromInt(e.effect);
  const color = colorFromInt(e.color);
  const speed = speedFromInt(e.speed);
  if (!effect || !color || !speed) return;
  const slot: AmbientSlot = {
    render: { effect, color, intensity: e.intensity, speed },
    expiresAt: e.autoClearMs > 0 ? Date.now() + e.autoClearMs : null,
    setAt: Date.now(),
  };
  ambientInternal.update((s) => {
    const next = new Map(s.byApp);
    next.set(e.appId, slot);
    return { byApp: next };
  });
}

function applyAmbientCleared(e: AmbientClearedEvent) {
  ambientInternal.update((s) => {
    const next = new Map(s.byApp);
    next.delete(e.appId);
    return { byApp: next };
  });
}

/// The effect on screen right now, and whose it is.
///
/// **Not the focused app's**, and that changed on 9 September. `ambient-api.md`
/// FA1 used to render only for the focused app, which meant the tint appeared
/// exactly when it had nothing to add - a focused window says its own state
/// better than a wash of colour - and stayed silent exactly when it did. Both
/// examples the API cites (a build running, tests failing) are things you want to
/// know while looking somewhere else. Ambient is peripheral awareness or it is
/// nothing.
///
/// **The most recently SET one wins**, not the first or the focused. One overlay
/// can show one effect, and the honest tie-break is recency: the app that just
/// said something is the one with news. In practice there is rarely a contest,
/// because the grant is the real bound - a tint is something a person hands to
/// one app at a time.
///
/// Carries the app id because a person seeing a tint has to be able to find out
/// whose it is. That is the attribution half of the same ruling.
export function pickLiveAmbient(
  byApp: Map<string, AmbientSlot>,
  now: number,
): { appId: string; render: AmbientRender } | null {
  let best: { appId: string; slot: AmbientSlot } | null = null;
  for (const [appId, slot] of byApp) {
    // Lazy-expire on read; the periodic prune below catches the rest.
    if (slot.expiresAt !== null && now > slot.expiresAt) continue;
    if (!best || slot.setAt >= best.slot.setAt) best = { appId, slot };
  }
  return best ? { appId: best.appId, render: best.slot.render } : null;
}

/// The live effect, or null. What the overlay draws.
export const liveAmbient: Readable<{ appId: string; render: AmbientRender } | null> = derived(
  ambientInternal,
  ($internal) => pickLiveAmbient($internal.byApp, Date.now()),
);

// ── Init ────────────────────────────────────────────────────────

/**
 * Wire Tauri-event listeners + the auto-clear pruner. Returns
 * a disposer matching the +layout init pattern.
 */
/// Forget everything this app published, because it has no windows any more.
///
/// PER-APP STATE OUTLIVES ITS APP. These three stores are keyed by app id and
/// nothing ever removed an entry except the app itself saying so - which a
/// window that crashed, was killed or logged out never gets to do. The comment
/// at the top of this file called that "stale entries from exited apps clear via
/// the same TTL / process-exit cleanup deferred for all per-app stores"; this is
/// that cleanup, and it is not a TTL.
///
/// OBSERVED ABSENCE, the same rule the knowledge daemon now uses on the other
/// side of the fence for an unclosed presence: not a timer that is wrong for
/// exactly its length and cannot tell a quiet app from a gone one, but the
/// question "is it still there". The shell's own instrument for that is its
/// window list rather than a pid.
///
/// Idempotent: an app with nothing stored is a no-op, so the caller may say this
/// as often as it likes.
export function forgetApp(appId: string): void {
  shortcutsInternal.update((s) => {
    if (!s.byApp.has(appId)) return s;
    const next = new Map(s.byApp);
    next.delete(appId);
    return { byApp: next };
  });
  badgesInternal.update((s) => {
    if (!s.byApp.has(appId)) return s;
    const next = new Map(s.byApp);
    next.delete(appId);
    return { byApp: next };
  });
  ambientInternal.update((s) => {
    if (!s.byApp.has(appId)) return s;
    const next = new Map(s.byApp);
    next.delete(appId);
    return { byApp: next };
  });
}

/// What these stores currently hold something for. The lifetime watcher reads it
/// to decide what has outlived its windows; nothing else should need it.
export function appsWithState(): string[] {
  const ids = new Set<string>();
  for (const k of get(shortcutsInternal).byApp.keys()) ids.add(k);
  for (const k of get(badgesInternal).byApp.keys()) ids.add(k);
  for (const k of get(ambientInternal).byApp.keys()) ids.add(k);
  return [...ids];
}

export function initAppStateStores(): () => void {
  const unlistens: UnlistenFn[] = [];
  const tasks = [
    listen<ShortcutRegisterEvent>("arlen://shortcut-register", (e) =>
      applyShortcutRegister(e.payload),
    ),
    listen<ShortcutStateEvent>("arlen://shortcut-state-changed", (e) =>
      applyShortcutStateChanged(e.payload),
    ),
    listen<ShortcutClearedEvent>("arlen://shortcut-cleared", (e) =>
      applyShortcutCleared(e.payload),
    ),
    listen<BadgeSetEvent>("arlen://badge-set", (e) => applyBadgeSet(e.payload)),
    listen<BadgeClearedEvent>("arlen://badge-cleared", (e) =>
      applyBadgeCleared(e.payload),
    ),
    listen<AmbientSetEvent>("arlen://ambient-set", (e) =>
      applyAmbientSet(e.payload),
    ),
    listen<AmbientClearedEvent>("arlen://ambient-cleared", (e) =>
      applyAmbientCleared(e.payload),
    ),
  ];

  Promise.all(tasks)
    .then((u) => unlistens.push(...u))
    .catch((e) => console.warn("initAppStateStores listen failed:", e));

  // Single auto-clear pruner for ambient. Polls every 1 s; drops
  // expired entries. Same loop covers all per-app stores if we
  // add TTL elsewhere later.
  const prunerHandle = setInterval(() => {
    ambientInternal.update((s) => {
      let changed = false;
      const now = Date.now();
      const next = new Map(s.byApp);
      for (const [appId, slot] of s.byApp) {
        if (slot.expiresAt !== null && now > slot.expiresAt) {
          next.delete(appId);
          changed = true;
        }
      }
      return changed ? { byApp: next } : s;
    });
  }, 1000);

  return () => {
    clearInterval(prunerHandle);
    for (const u of unlistens) {
      try {
        u();
      } catch {
        // swallow
      }
    }
  };
}
