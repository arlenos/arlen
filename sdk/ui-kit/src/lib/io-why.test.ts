// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The errno reader, against the texts a host really sends: each lands on its
// key, and an unknown one yields nothing rather than the host's words.
import { describe, it, expect, vi } from "vitest";
import { ioWhyKey } from "./io-why.js";
import { kitMessages } from "./i18n/messages.kit.js";

describe("ioWhyKey", () => {
  it("reads the common errno texts", () => {
    expect(ioWhyKey("Permission denied (os error 13)")).toBe("k.why.permission");
    expect(ioWhyKey("Read-only file system (os error 30)")).toBe("k.why.readOnly");
    expect(ioWhyKey("No space left on device (os error 28)")).toBe("k.why.noSpace");
    expect(ioWhyKey("No such file or directory (os error 2)")).toBe("k.why.gone");
    expect(ioWhyKey("Is a directory (os error 21)")).toBe("k.why.notAFile");
  });

  it("yields nothing for an unknown text or none", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    expect(ioWhyKey("Input/output error (os error 5)")).toBeNull();
    expect(ioWhyKey(null)).toBeNull();
    warn.mockRestore();
  });

  it("has a sentence for every key in both locales", () => {
    for (const lang of ["en", "de"] as const) {
      const table = kitMessages[lang] as Record<string, string>;
      for (const key of ["k.why.permission", "k.why.readOnly", "k.why.noSpace", "k.why.gone", "k.why.notAFile"]) {
        expect(table[key], `${lang} ${key}`).toBeTruthy();
      }
    }
  });
});
