/// Revoking a grant has to take back what the grant was doing.
///
/// The gap this covers is one a person could see: take `ambient` away from an
/// app on its Settings page and its tint stayed on the screen until the app
/// exited, because the shell's per-app stores only ever dropped an entry when
/// the app said so or when its windows went. The instrument somebody actually
/// has for a running effect did not take it back.

import { describe, expect, it, vi, beforeEach } from "vitest";
import { writable } from "svelte/store";

const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));

const forgetApp = vi.fn();
const forgetMenu = vi.fn();
const forgetToolbarApp = vi.fn();
vi.mock("./windows", () => ({ windows: writable([]) }));
vi.mock("./activeApp", () => ({ resolvePermissionId: async (id: string) => id }));
vi.mock("./appStateStores", () => ({ forgetApp, appsWithState: () => [] }));
vi.mock("./menus", () => ({ forgetMenu, appsWithMenus: () => [] }));
vi.mock("./toolbarStore", () => ({ forgetToolbarApp, appsWithToolbar: () => [] }));

const { initPermissionForget } = await import("./appStateLifetime");
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(async () => {
  forgetApp.mockClear();
  forgetMenu.mockClear();
  forgetToolbarApp.mockClear();
  initPermissionForget();
  await settle();
});

describe("initPermissionForget", () => {
  /// Everything the app published, not only the surface whose grant changed:
  /// the shell does not read capability files and should not start. Whatever
  /// the app still may publish, it publishes again.
  it("gives up everything the app published", () => {
    handlers["arlen://permission-changed"]?.({
      payload: { appId: "dev.arlen.terminal", exists: true },
    });
    expect(forgetApp).toHaveBeenCalledWith("dev.arlen.terminal");
    expect(forgetMenu).toHaveBeenCalledWith("dev.arlen.terminal");
    expect(forgetToolbarApp).toHaveBeenCalledWith("dev.arlen.terminal");
  });

  /// A payload with no app names nobody. Dropping on it would clear the board
  /// for every app at once, which is the shape that made the window-absence
  /// watcher need its own guard.
  it("does nothing for a change that names no app", () => {
    handlers["arlen://permission-changed"]?.({ payload: {} });
    expect(forgetApp).not.toHaveBeenCalled();
  });
});
