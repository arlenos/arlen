/// A failed read must stay distinguishable from an empty answer, on the one
/// screen with nowhere to put a second message.
///
/// Every function here has a catch, and each catch is load-bearing in a
/// different way. `listProfiles` returning `[]` instead of `null` would render
/// as "this machine has no users" - a sentence the greeter cannot know is true.
/// `power` returning `true` on a refusal is worse: the menu closes, the machine
/// stays on, and someone walks away from it. That one used to swallow the
/// failure on the reasoning that a login screen has nothing to surface, which is
/// why the return value is worth pinning rather than trusting.
///
/// These are the shapes the surface branches on, so they are tested at the
/// boundary the surface sees.

import { describe, expect, it, vi } from "vitest";

const invoke = vi.fn((..._args: unknown[]) => Promise.resolve(null as unknown));
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

const { listProfiles, listSessions, authenticate, beginFactor, readWallpaper, power } =
  await import("./greeter");

// No `beforeEach` reset. Both `mockReset` and `mockClear` make a rejection set
// after them escape the code's own catch here; reproduced in isolation, the same
// single test passes with neither and fails with either. Each test sets the
// implementation it needs, which is all this file wants anyway.

describe("reads", () => {
  it("hands back the profiles the backend gave", async () => {
    invoke.mockResolvedValue([{ id: "tim", name: "Tim" }]);
    expect(await listProfiles()).toHaveLength(1);
  });

  it("answers a failed profile read with null, not an empty list", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("greetd is not there")));
    // `[]` would be rendered as "no users on this machine", which is a claim
    // about the machine rather than about the read.
    expect(await listProfiles()).toBeNull();
  });

  it("answers a failed session read with null, not an empty list", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("no sessions socket")));
    expect(await listSessions()).toBeNull();
  });

  it("falls back to no wallpaper rather than failing the screen", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("no manifest")));
    expect(await readWallpaper()).toBeNull();
  });
});

describe("authentication", () => {
  it("carries the backend's own verdict through", async () => {
    invoke.mockResolvedValue({ ok: true });
    expect(await authenticate("tim", "secret", "session")).toEqual({ ok: true });
  });

  /// The rejection carries a TOKEN now, not prose: the host answers `no-greetd`,
  /// `unknown-profile` and their kin, and `AuthPanel` turns each into a sentence
  /// in the reader's language. This test carries the token through unchanged,
  /// because the panel is what decides the wording and it cannot do that if this
  /// layer rewrites or drops what it was told.
  it("carries a refusal token through for the panel to word", async () => {
    invoke.mockImplementation(() => Promise.reject("no-greetd"));
    const r = await authenticate("tim", "wrong", "session");
    expect(r.ok).toBe(false);
    expect(r.error).toBe("no-greetd");
  });

  it("does the same for a hardware factor", async () => {
    invoke.mockImplementation(() => Promise.reject("unknown-profile"));
    const r = await beginFactor("tim", "fido2");
    expect(r.ok).toBe(false);
    expect(r.error).toBe("unknown-profile");
  });

  it("passes the screen-reader state as null when nobody touched the toggle", async () => {
    invoke.mockResolvedValue({ ok: true });
    await authenticate("tim", "secret", "session");
    // Not `false`: that would overwrite the user's own setting with a default
    // they never chose, and the greeter cannot read their config to know better.
    expect(invoke.mock.calls.at(-1)?.[1]).toMatchObject({ screenReader: null });
  });
});

describe("power", () => {
  it("reports an accepted action", async () => {
    invoke.mockResolvedValue(null);
    expect(await power("power-off")).toBe(true);
  });

  it("reports a refused one instead of swallowing it", async () => {
    invoke.mockImplementation(() => Promise.reject(new Error("logind refused")));
    // The regression this guards: it returned nothing at all, so the caller had
    // no way to say "that did not happen, the machine is still on".
    expect(await power("power-off")).toBe(false);
  });
});
