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
    expect(get(mod.focusedAmbient)).toBeNull();
  });
});
