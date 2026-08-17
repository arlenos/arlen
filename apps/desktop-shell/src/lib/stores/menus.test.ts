/// A registered menu must be found for the window that owns it.
///
/// The pair this and `activeApp.test.ts` form is the whole of the 17 August
/// defect: the resolver turns the window's id into the app's, and this checks
/// that the menu lookup is keyed on the resolved one. Before the fix the topbar
/// asked `appMenus` for `arlen-knowledge` while every menu was registered under
/// `dev.arlen.knowledge`, so it found nothing, for every app, always.

import { describe, expect, it, beforeEach, vi } from "vitest";
import { get, writable } from "svelte/store";

const activeAppId = writable<string | null>(null);
vi.mock("./activeApp.js", () => ({ activeAppId }));

/// Captured Tauri event handlers, so a test can deliver a registration the way
/// the backend does.
const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));

/// `fetchMenu` asks the backend store on focus; it has nothing to add here.
vi.mock("@tauri-apps/api/core", () => ({ invoke: () => Promise.resolve(null) }));

const { activeMenu, initMenuListeners } = await import("./menus");

const GROUPS = [{ label: "File", items: [] }];
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  activeAppId.set(null);
  initMenuListeners();
  await settle();
  handlers["arlen://menu-registered"]?.({
    payload: { app_id: "dev.arlen.knowledge", items: GROUPS },
  });
});

describe("activeMenu", () => {
  it("finds the menu when the focused window resolved to the app that registered it", async () => {
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    expect(get(activeMenu)).toEqual(GROUPS);
  });

  it("finds nothing for an app that registered no menu", async () => {
    activeAppId.set("dev.arlen.files");
    await settle();
    expect(get(activeMenu)).toBeNull();
  });

  it("is null with nothing focused", async () => {
    activeAppId.set(null);
    await settle();
    expect(get(activeMenu)).toBeNull();
  });

  it("drops the menu again when the app unregisters it", async () => {
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    handlers["arlen://menu-unregistered"]?.({ payload: { app_id: "dev.arlen.knowledge" } });
    await settle();
    expect(get(activeMenu)).toBeNull();
  });
});
