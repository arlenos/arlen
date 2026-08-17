/// The app half of the menu chain: publish under our own id, and act only on
/// actions addressed to us.
///
/// The shell half was the 17 August defect - the topbar looked a menu up under
/// the focused window's id while it was registered under the permission id. This
/// side has the mirror hazard: `initAppMenu` filters incoming actions on a
/// hardcoded `APP_ID`, so if that string ever drifts from what the app publishes
/// under, every menu click is dropped in silence. Nothing else would notice: the
/// menu still renders, the click still happens, and nothing runs.

import { describe, expect, it, vi } from "vitest";

const invoke = vi.fn(() => Promise.resolve(null));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const handlers: Record<string, (e: { payload: unknown }) => void> = {};
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers[name] = cb;
    return Promise.resolve(() => {});
  },
}));

const { initAppMenu } = await import("./menu");
const { pendingMenuAction } = await import("$lib/stores/timeline");
const { get } = await import("svelte/store");

const settle = () => new Promise((r) => setTimeout(r, 0));

/// The groups the app handed the shell, from the registration call it made.
function registered(): Array<{ items: Array<{ action: string; type: string }> }> {
  const call = invoke.mock.calls.find((c) => c[0] === "plugin:arlen-shell|menu_register");
  return (call?.[1] as { groups: never[] })?.groups ?? [];
}

describe("initAppMenu", () => {
  it("registers through the shell plugin, not the shell's own command name", async () => {
    await initAppMenu();
    await settle();
    // `register_menu` is compiled into the shell's binary; an app calling it by
    // that name is rejected at runtime. The plugin spelling is the app-facing one.
    const names = invoke.mock.calls.map((c) => c[0]);
    expect(names).toContain("plugin:arlen-shell|menu_register");
    expect(names).not.toContain("register_menu");
  });

  it("routes an action addressed to this app", async () => {
    await initAppMenu();
    await settle();
    pendingMenuAction.set(null);
    handlers["arlen://menu-action"]?.({
      payload: { app_id: "dev.arlen.knowledge", action: "timeline.export" },
    });
    expect(get(pendingMenuAction)).toBe("export");
  });

  it("ignores an action addressed to another app", async () => {
    await initAppMenu();
    await settle();
    pendingMenuAction.set(null);
    handlers["arlen://menu-action"]?.({
      payload: { app_id: "dev.arlen.files", action: "timeline.export" },
    });
    expect(get(pendingMenuAction)).toBeNull();
  });

  it("has no menu item whose action nothing handles", async () => {
    await initAppMenu();
    await settle();
    const actions = registered()
      .flatMap((g) => g.items)
      .filter((i) => i.type !== "separator")
      .map((i) => i.action);
    expect(actions.length).toBeGreaterThan(0);
    for (const action of actions) {
      pendingMenuAction.set(null);
      handlers["arlen://menu-action"]?.({
        payload: { app_id: "dev.arlen.knowledge", action },
      });
      expect(get(pendingMenuAction), `${action} is offered but handled by nothing`).not.toBeNull();
    }
  });
});
