// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// @vitest-environment jsdom
/// A colour dot is a mark when nobody presses it and a named button when
/// somebody does. The mark stays out of a reader's way, since the label beside
/// it says what it is; the button carries the colour's name in words and says
/// which one is chosen, so a picker is not read out as a list of hex values.
import { describe, expect, it, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import ColorDot from "./color-dot.svelte";

describe("color dot", () => {
  it("is a hidden mark without an action", () => {
    const { container } = render(ColorDot, { color: "#7aa2f7" });
    const el = container.querySelector(".color-dot")!;
    expect(el.tagName).toBe("SPAN");
    expect(el.getAttribute("aria-hidden")).toBe("true");
  });

  it("is a named, pressed-state button with one", async () => {
    const onclick = vi.fn();
    const { container } = render(ColorDot, { color: "#7aa2f7", size: "pick", label: "Blue", selected: true, onclick });
    const b = container.querySelector("button")!;
    expect(b.getAttribute("aria-label")).toBe("Blue");
    expect(b.getAttribute("aria-pressed")).toBe("true");
    await fireEvent.click(b);
    expect(onclick).toHaveBeenCalledOnce();
  });
});
