// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

// @vitest-environment jsdom
/// The dialog's transitions read their duration from the theme, and jsdom has
/// no theme and no Web Animations API. What must hold there is what holds
/// under reduce motion: the dialog is there at once when opened and gone at
/// once when closed, with nothing left half-mounted.
import { describe, expect, it } from "vitest";
import { render } from "@testing-library/svelte";
import { createRawSnippet, tick } from "svelte";
import Dialog from "./dialog.svelte";

const body = createRawSnippet(() => ({ render: () => "<p>Delete permanently?</p>" }));

describe("dialog", () => {
  it("is there when opened and gone when closed, with no duration to wait for", async () => {
    const { rerender } = render(Dialog, { open: true, onClose: () => {}, ariaLabel: "example", children: body });
    expect(document.querySelector("[role=dialog]")).not.toBeNull();
    await rerender({ open: false, onClose: () => {}, ariaLabel: "example", children: body });
    await tick();
    expect(document.querySelector("[role=dialog]")).toBeNull();
    expect(document.querySelector(".dialog-backdrop")).toBeNull();
  });
});
