// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The refusal reader, against the sentences the host really sends: each one
// lands on its cause, and anything else lands on "other" rather than on a
// person's screen.
import { describe, it, expect, vi } from "vitest";
import { causeOf } from "./refusals.js";

describe("causeOf", () => {
  it("reads the Tauri layer's preflight sentences", () => {
    expect(causeOf("org.example.timer is not installed")).toBe("notInstalled");
    expect(causeOf("org.example.timer is recorded as installed from apt, which this build has no way to remove")).toBe("layerRemove");
    expect(causeOf("org.example.timer is recorded as installed from apt, which this build has no way to update")).toBe("layerUpdate");
    expect(causeOf("org.example.x was installed as a package file, and updating that layer needs one nothing on this machine fetches yet")).toBe("packageFile");
  });

  it("reads installd's refusals through their D-Bus wrapper", () => {
    expect(causeOf("org.freedesktop.DBus.Error.AccessDenied: org.arlen.shell is part of the desktop itself and cannot be removed")).toBe("notRemovable");
    expect(causeOf("org.freedesktop.DBus.Error.AccessDenied: dev.arlen.files may not change what is installed")).toBe("notAllowed");
    expect(causeOf("org.freedesktop.DBus.Error.AccessDenied: resolve caller: unknown binary path: /usr/bin/x")).toBe("notAllowed");
  });

  it("reads a missing service and a skip that did not land", () => {
    expect(causeOf("store transport error: Connection refused (os error 111)")).toBe("noDaemon");
    expect(causeOf("org.freedesktop.DBus.Error.ServiceUnknown: The name org.arlen.Install1 was not provided")).toBe("noDaemon");
    expect(causeOf("no data directory to record the skip in")).toBe("skipNotRecorded");
    expect(causeOf("/home/x/.local/share/arlen/store/skips.toml.tmp: Permission denied (os error 13)")).toBe("skipNotRecorded");
    expect(causeOf("the catalogue is too large to send in one response: 17000000 bytes, and the limit is 16777216")).toBe("tooLarge");
  });

  it("sends the rest to the console, not the screen", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(causeOf("unexpected store response: Cards([])")).toBe("other");
    expect(causeOf({ some: "object" })).toBe("other");
    expect(causeOf(new Error("boom"))).toBe("other");
    expect(warn).toHaveBeenCalledTimes(3);
    warn.mockRestore();
  });
});
