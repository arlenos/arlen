// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// @vitest-environment jsdom
/// A chip is a label, and a remove control when asked for one. The remove
/// control is named after the chip in the reader's language: the kit's two
/// earlier chips both said "Remove <x>" in English to a German reader.
import { describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";
import { render, fireEvent } from "@testing-library/svelte";
import { locale } from "../../../i18n";
import Chip from "./chip.svelte";

describe("chip", () => {
  it("is a plain label without a remove control", () => {
    const { container } = render(Chip, { label: "Documents" });
    expect(container.querySelector(".chip-label")?.textContent).toBe("Documents");
    expect(container.querySelector("button")).toBeNull();
  });

  it("names its remove control in the reader's language and calls back", async () => {
    const before = get(locale);
    locale.set("de");
    try {
      const onremove = vi.fn();
      const { container } = render(Chip, { label: "Dokumente", onremove });
      const x = container.querySelector(".chip-x")!;
      // The label arrives bidi-isolated (U+2068 … U+2069), as every interpolated value does.
      expect(x.getAttribute("aria-label")?.replace(/[\u2068\u2069]/g, "")).toBe("Dokumente entfernen");
      await fireEvent.click(x);
      expect(onremove).toHaveBeenCalledOnce();
    } finally {
      locale.set(before);
    }
  });

  it("is a filter toggle with pressed, its id on the button", () => {
    const { container } = render(Chip, { label: "Verified", id: "facet-verified", pressed: true, onclick: () => {} });
    const b = container.querySelector("button")!;
    expect(b.id).toBe("facet-verified");
    expect(b.getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector(".chip")?.classList.contains("on")).toBe(true);
  });
});
