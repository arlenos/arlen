// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// @vitest-environment jsdom
/// A button's hover label is the kit tooltip, never the browser's: with
/// `tooltip` set there is no `title` on the element, the accessible name the
/// caller gave is untouched, and a press still reaches the caller's handler
/// through the tooltip trigger's own.
import { describe, expect, it, vi } from "vitest";
import { render, fireEvent } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import Button from "./button.svelte";

const icon = createRawSnippet(() => ({ render: () => "<span>x</span>" }));

describe("button tooltip", () => {
  it("labels through the kit, not a title, and still presses", async () => {
    const onclick = vi.fn();
    const { container } = render(Button, { tooltip: "Undo", "aria-label": "Undo", onclick, children: icon });
    const b = container.querySelector("button")!;
    expect(b.hasAttribute("title")).toBe(false);
    expect(b.getAttribute("aria-label")).toBe("Undo");
    await fireEvent.click(b);
    expect(onclick).toHaveBeenCalledTimes(1);
  });

  it("renders plainly without one", () => {
    const { container } = render(Button, { children: icon });
    expect(container.querySelector("button")?.hasAttribute("data-state")).toBe(false);
  });
});
