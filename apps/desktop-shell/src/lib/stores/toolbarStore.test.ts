/// The toolbar must render for the app that published it.
///
/// The last of the four surfaces that went dark on 17 August. This one is keyed
/// per (app, window) rather than per app, so the fix threaded the resolved
/// permission id in beside the window's own id - the app half comes from
/// `activeApp`, the window half stays the compositor's toplevel id.

import { describe, expect, it, beforeEach, vi } from "vitest";
import { get, writable } from "svelte/store";

const activeAppId = writable<string | null>(null);
const activeWindow = writable<{ app_id: string; id: string } | null>(null);
vi.mock("./activeApp", () => ({ activeAppId }));
vi.mock("./windows", () => ({ activeWindow }));

const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));

const { focusedToolbar, focusedToolbarKey, initToolbarStore } = await import("./toolbarStore");

const ACTIONS = [{ icon: "save", action: "save", tooltip: "Save", toggle: false, active: false }];
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  activeAppId.set(null);
  activeWindow.set(null);
  initToolbarStore();
  await settle();
});

describe("focusedToolbar", () => {
  it("renders the toolbar the focused app published", async () => {
    handlers["arlen://toolbar-quick-actions"]?.({
      payload: { appId: "dev.arlen.knowledge", windowId: "w1", actions: ACTIONS },
    });
    activeWindow.set({ app_id: "arlen-knowledge", id: "w1" });
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    expect(get(focusedToolbar).kind).toBe("quick-actions");
  });

  it("keys on the permission id for the app and the toplevel id for the window", async () => {
    activeWindow.set({ app_id: "arlen-knowledge", id: "w1" });
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    expect(get(focusedToolbarKey)).toEqual({ appId: "dev.arlen.knowledge", windowId: "w1" });
  });

  it("shows nothing for an app that published no toolbar", async () => {
    handlers["arlen://toolbar-quick-actions"]?.({
      payload: { appId: "dev.arlen.knowledge", windowId: "w1", actions: ACTIONS },
    });
    activeWindow.set({ app_id: "arlen-files", id: "w2" });
    activeAppId.set("dev.arlen.files");
    await settle();
    expect(get(focusedToolbar).kind).toBe("none");
  });

  it("clears when the app withdraws it", async () => {
    handlers["arlen://toolbar-quick-actions"]?.({
      payload: { appId: "dev.arlen.knowledge", windowId: "w1", actions: ACTIONS },
    });
    activeWindow.set({ app_id: "arlen-knowledge", id: "w1" });
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    handlers["arlen://toolbar-cleared"]?.({
      payload: { appId: "dev.arlen.knowledge", windowId: "w1" },
    });
    await settle();
    expect(get(focusedToolbar).kind).toBe("none");
  });
});
