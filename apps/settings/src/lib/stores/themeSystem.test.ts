/// The system page must show the theme's terminal palette, and keep showing it.
///
/// `resolvedDefaults` is one module-level store that two pages fill from two
/// commands: the system page reads `theme_resolved_terminal`, the sound page
/// reads `theme_resolved_sounds`. The sound loader replaced the whole store
/// instead of merging into it, so opening Appearance > Sound discarded the
/// eighteen terminal values and the grid fell back to `SYS_DEFAULTS` - a
/// hardcoded copy of a palette the shipped theme has since moved away from
/// (its blue is `#7d9cc4`, the copy's is `#2563eb`). Going back to the system
/// page re-read them, which is why it looked fine from any single page.

import { describe, expect, it, vi, beforeEach } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { loadResolvedTerminal, loadResolvedSounds, effective, overrides } =
  await import("./themeSystem");
const { get } = await import("svelte/store");

const THEME_BLUE = "#7d9cc4";

function backend() {
  invoke.mockReset();
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "theme_resolved_terminal") {
      return Promise.resolve({
        fg: "#c8c9cf",
        bg: "#0f0f0f",
        cursor: "#8fae74",
        ansi: ["#15161b", "#c96a6a", "#8fae74", "#d4b483", THEME_BLUE],
      });
    }
    if (cmd === "theme_resolved_sounds") {
      return Promise.resolve([{ event: "notification", sound: "message" }]);
    }
    return Promise.reject(new Error("unexpected command " + cmd));
  });
}

beforeEach(() => {
  overrides.set({});
  backend();
});

describe("the resolved reads", () => {
  it("shows the theme's blue rather than the hardcoded copy", async () => {
    await loadResolvedTerminal();
    expect(get(effective).ansi4).toBe(THEME_BLUE);
    expect(get(effective).termBg).toBe("#0f0f0f");
  });

  it("keeps the terminal palette when the sound page loads after it", async () => {
    await loadResolvedTerminal();
    await loadResolvedSounds();
    expect(get(effective).ansi4).toBe(THEME_BLUE);
    expect(get(effective).sndNotification).toBe("message");
  });

  it("keeps the cues when the system page loads after them", async () => {
    await loadResolvedSounds();
    await loadResolvedTerminal();
    expect(get(effective).sndNotification).toBe("message");
    expect(get(effective).ansi4).toBe(THEME_BLUE);
  });

  it("leaves a slot the backend did not send on the hardcoded floor", async () => {
    // Five slots arrive; the other eleven must keep a value rather than blank a
    // swatch. A short list is a partial answer, not a reason to show nothing.
    await loadResolvedTerminal();
    expect(get(effective).ansi15).toBeTruthy();
  });

  it("leaves the floor in place when the backend will not answer", async () => {
    invoke.mockReset();
    invoke.mockImplementation(() => Promise.reject(new Error("no backend")));
    await loadResolvedTerminal();
    expect(get(effective).ansi4).toBeTruthy();
  });
});
