/// The three per-app surfaces must key on the id their state is published under.
///
/// Shortcuts, badges and ambient arrive from `app.shortcut.*`, `app.badge.*` and
/// `app.ambient.*`, all keyed by the publishing app's permission id, and all three
/// looked their state up under the focused window's own id until 17 August. So a
/// correctly published shortcut list, badge and ambient effect were invisible at
/// once, for every app - the same single assumption in three more places.

import { describe, expect, it, beforeEach, vi } from "vitest";
import { get, writable } from "svelte/store";

const activeAppId = writable<string | null>(null);
vi.mock("./activeApp", () => ({ activeAppId }));

const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));

const mod = await import("./appStateStores");
const { pickLiveAmbient } = mod;
const settle = () => new Promise((r) => setTimeout(r, 0));

/// Start every listener the module offers, whatever they are called, so the test
/// does not have to track which init belongs to which family.
beforeEach(async () => {
  activeAppId.set(null);
  for (const [name, fn] of Object.entries(mod)) {
    if (typeof fn === "function" && /^init/.test(name)) (fn as () => unknown)();
  }
  await settle();
});

describe("the per-app surfaces", () => {
  it("shows a shortcut list to the app that registered it, and to no other", async () => {
    handlers["arlen://shortcut-register"]?.({
      payload: { appId: "dev.arlen.knowledge", shortcuts: [{ label: "Search", icon: "search", action: "search" }] },
    });
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    expect(get(mod.focusedShortcuts)).toHaveLength(1);

    activeAppId.set("dev.arlen.files");
    await settle();
    expect(get(mod.focusedShortcuts)).toHaveLength(0);
  });

  it("gives nothing when nothing is focused", async () => {
    handlers["arlen://shortcut-register"]?.({
      payload: { appId: "dev.arlen.knowledge", shortcuts: [{ label: "Search", icon: "search", action: "search" }] },
    });
    activeAppId.set(null);
    await settle();
    expect(get(mod.focusedShortcuts)).toHaveLength(0);
    expect(get(mod.focusedBadge)).toBeNull();
  });

  /// The one surface that is deliberately NOT about the focused app any more.
  /// A shortcut list and a badge belong to the window in front of you; an
  /// ambient effect is the opposite - it is worth showing precisely while you
  /// are looking somewhere else, which is what dropping FA1's focused-only
  /// clause was for.
  it("keeps showing an ambient effect while its app is not the focused one", async () => {
    handlers["arlen://ambient-set"]?.({
      payload: {
        appId: "dev.arlen.terminal",
        effect: 1,
        color: 1,
        intensity: 0.2,
        speed: 1,
        autoClearMs: 0,
      },
    });
    activeAppId.set("dev.arlen.files");
    await settle();
    expect(get(mod.liveAmbient)?.appId).toBe("dev.arlen.terminal");

    activeAppId.set(null);
    await settle();
    expect(get(mod.liveAmbient)?.appId).toBe("dev.arlen.terminal");
  });

  it("stops showing it when its app clears it", async () => {
    handlers["arlen://ambient-set"]?.({
      payload: {
        appId: "dev.arlen.terminal",
        effect: 1,
        color: 1,
        intensity: 0.2,
        speed: 1,
        autoClearMs: 0,
      },
    });
    await settle();
    handlers["arlen://ambient-cleared"]?.({ payload: { appId: "dev.arlen.terminal" } });
    await settle();
    expect(get(mod.liveAmbient)).toBeNull();
  });
});

describe("pickLiveAmbient", () => {
  const slot = (setAt: number, expiresAt: number | null = null) => ({
    render: {
      effect: "pulse" as const,
      color: "accent" as const,
      intensity: 0.2,
      speed: "slow" as const,
    },
    expiresAt,
    setAt,
  });

  /// One overlay can show one effect, and the honest tie-break is recency: the
  /// app that just said something is the one with news. Insertion order will not
  /// do it - a Map keeps a re-set key in its ORIGINAL position, so the app that
  /// spoke first would keep the screen for as long as it kept speaking.
  it("shows the most recently set effect, not the first", () => {
    const byApp = new Map([
      ["a", slot(100)],
      ["b", slot(200)],
    ]);
    expect(pickLiveAmbient(byApp, 300)?.appId).toBe("b");
    byApp.set("a", slot(400));
    expect(pickLiveAmbient(byApp, 500)?.appId).toBe("a");
  });

  it("passes over one that has expired", () => {
    const byApp = new Map([
      ["a", slot(100, 150)],
      ["b", slot(50)],
    ]);
    expect(pickLiveAmbient(byApp, 200)?.appId).toBe("b");
  });

  it("says nothing when every effect has expired", () => {
    expect(pickLiveAmbient(new Map([["a", slot(100, 150)]]), 200)).toBeNull();
  });

  it("says nothing when nothing is set", () => {
    expect(pickLiveAmbient(new Map(), 1)).toBeNull();
  });
});
