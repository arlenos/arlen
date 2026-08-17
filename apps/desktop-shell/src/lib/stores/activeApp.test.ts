/// The focused window must resolve to the id its app publishes under.
///
/// This is the test that would have caught the 17 August defect. A window
/// announces `arlen-knowledge`, matching its `.desktop` file; the app registers
/// its menu, toolbar, shortcuts, badge and ambient state under
/// `dev.arlen.knowledge`, the id the permission system resolves. Keying the
/// lookup on the window's own id found nothing, so five top-bar surfaces were
/// dead for every app at once - and nothing in this crate could have noticed,
/// because it had no frontend tests at all.

import { describe, expect, it, vi } from "vitest";
import { get, writable } from "svelte/store";

/// The focused window, as `windows.ts` would publish it.
const activeWindow = writable<{ app_id: string; id: string } | null>(null);
vi.mock("./windows", () => ({ activeWindow }));

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { activeAppId } = await import("./activeApp");

/// The app index's answer: the `.desktop` file's `X-Arlen-AppId`.
function resolvesAs(map: Record<string, string>) {
  invoke.mockReset();
  invoke.mockImplementation((cmd: string, args: { windowAppId: string }) => {
    if (cmd !== "resolve_app_id") return Promise.reject(new Error("unexpected " + cmd));
    return Promise.resolve(map[args.windowAppId] ?? args.windowAppId);
  });
}

/// Let the resolution round trip settle.
const settle = () => new Promise((r) => setTimeout(r, 0));

describe("activeAppId", () => {
  it("resolves a window id to the permission id its app publishes under", async () => {
    resolvesAs({ "arlen-knowledge": "dev.arlen.knowledge" });
    activeWindow.set({ app_id: "arlen-knowledge", id: "w1" });
    await settle();
    expect(get(activeAppId)).toBe("dev.arlen.knowledge");
  });

  it("keeps the window's own id when no installed app claims it", async () => {
    // A third-party window announces whatever its toolkit sets. Falling back to
    // that is right: its state goes missing rather than being attributed to
    // another app.
    resolvesAs({});
    activeWindow.set({ app_id: "org.mozilla.firefox", id: "w2" });
    await settle();
    expect(get(activeAppId)).toBe("org.mozilla.firefox");
  });

  it("falls back to the window id when the index cannot answer", async () => {
    invoke.mockReset();
    invoke.mockImplementation(() => Promise.reject(new Error("no index")));
    activeWindow.set({ app_id: "arlen-files", id: "w3" });
    await settle();
    expect(get(activeAppId)).toBe("arlen-files");
  });

  it("is null when nothing is focused", async () => {
    resolvesAs({});
    activeWindow.set(null);
    await settle();
    expect(get(activeAppId)).toBeNull();
  });
});
