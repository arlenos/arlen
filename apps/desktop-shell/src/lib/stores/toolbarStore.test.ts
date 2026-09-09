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

const { focusedToolbar, focusedToolbarKey, initToolbarStore, forgetToolbarApp, appsWithToolbar } =
  await import("./toolbarStore");

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

  /// The teardown an app that died never got to send. `toolbar_clear` is on the
  /// plugin and nothing calls it; an app that is killed could not call it
  /// anyway, so the shell reclaims on observed absence instead.
  it("forgets every window an app published under, not just the focused one", async () => {
    for (const windowId of ["main", "second"]) {
      handlers["arlen://toolbar-quick-actions"]?.({
        payload: { appId: "dev.arlen.knowledge", windowId, actions: ACTIONS },
      });
    }
    handlers["arlen://toolbar-quick-actions"]?.({
      payload: { appId: "dev.arlen.files", windowId: "main", actions: ACTIONS },
    });
    await settle();
    expect(appsWithToolbar().sort()).toEqual(["dev.arlen.files", "dev.arlen.knowledge"]);

    forgetToolbarApp("dev.arlen.knowledge");
    await settle();
    expect(appsWithToolbar()).toEqual(["dev.arlen.files"]);

    // And the gone app cannot lend a toolbar to a window that opens later. The
    // fallback below `focusedToolbar` returns ANY state under the focused app,
    // so an entry left behind is not merely stored, it renders.
    activeWindow.set({ app_id: "arlen-knowledge", id: "w9" });
    activeAppId.set("dev.arlen.knowledge");
    await settle();
    expect(get(focusedToolbar).kind).toBe("none");
  });

  it("leaves another app's toolbar alone", async () => {
    handlers["arlen://toolbar-quick-actions"]?.({
      payload: { appId: "dev.arlen.files", windowId: "main", actions: ACTIONS },
    });
    await settle();
    forgetToolbarApp("dev.arlen.knowledge");
    activeWindow.set({ app_id: "arlen-files", id: "w3" });
    activeAppId.set("dev.arlen.files");
    await settle();
    expect(get(focusedToolbar).kind).toBe("quick-actions");
  });
});
