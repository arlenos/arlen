// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The host says why a read came back empty in a sentence of its own, and names
// a process state with a word of its own. Neither reaches the screen: these two
// readers turn the finite set the host sends into catalogue keys, and anything
// else into the general word rather than the host's text.
import { describe, it, expect, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => ({}) }));

import { readUnread, stateKey } from "./detail.js";

describe("readUnread", () => {
  it("reads the sentences the host sends", () => {
    expect(readUnread("not measured: the process has exited")).toBe("exited");
    expect(readUnread("not measured: this process belongs to another user")).toBe("otherUser");
    expect(readUnread("not measured: its status could not be read")).toBe("status");
    expect(readUnread("not measured: its file table could not be read")).toBe("files");
  });

  it("keeps nothing for a measured read and the general word for the rest", () => {
    expect(readUnread(null)).toBeNull();
    expect(readUnread(undefined)).toBeNull();
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(readUnread("not measured: something new")).toBe("other");
    warn.mockRestore();
  });
});

describe("stateKey", () => {
  it("names the host's state words in the catalogue", () => {
    expect(stateKey("Running")).toBe("tm.dp.state.running");
    expect(stateKey("Waiting for disk")).toBe("tm.dp.state.disk");
    expect(stateKey("Stopped by a debugger")).toBe("tm.dp.state.debugger");
    expect(stateKey("Unknown")).toBe("tm.dp.state.unknown");
    expect(stateKey("Zombie")).toBe("tm.dp.state.unknown");
  });
});
