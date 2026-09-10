// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The quiet-chrome rule in code: a block that succeeded gets NOTHING, and the
// absence of a chip is the status. It is one `if` and it is the difference
// between a terminal that stays a terminal and one that grows a badge under
// every command a person runs.
import { describe, it, expect, beforeEach } from "vitest";
import { applyBlockHover, renderBlockResult } from "./block-chrome.js";

let el: HTMLElement;

beforeEach(() => {
  document.body.innerHTML = "";
  el = document.createElement("div");
  document.body.append(el);
});

describe("applyBlockHover", () => {
  it("marks the block, and warms it only for a failure", () => {
    applyBlockHover(el);
    expect(el.classList.contains("arlen-block-hover")).toBe(true);
    expect(el.classList.contains("is-error")).toBe(false);

    applyBlockHover(el, { isError: true });
    expect(el.classList.contains("is-error")).toBe(true);
  });

  it("takes the warmth back when the same element is reused", () => {
    // The engine hands the same element back as a block's state changes; a class
    // that only ever goes on would leave a successful block wearing the error
    // wash.
    applyBlockHover(el, { isError: true });
    applyBlockHover(el, { isError: false });
    expect(el.classList.contains("is-error")).toBe(false);
  });
});

describe("renderBlockResult", () => {
  it("says nothing at all about a command that worked", () => {
    renderBlockResult(el, { exitCode: 0 });
    expect(el.textContent).toBe("");
    expect(el.children).toHaveLength(0);
  });

  it("says nothing about a command whose exit is not known yet", () => {
    renderBlockResult(el, { exitCode: null });
    expect(el.children).toHaveLength(0);
  });

  it("names the code when a command failed", () => {
    renderBlockResult(el, { exitCode: 130 });
    const chip = el.querySelector(".arlen-block-exit");
    expect(chip?.textContent).toBe("exit 130");
  });

  it("replaces what was there rather than stacking chips", () => {
    // Re-render is the normal case - xterm calls `onRender` again - and a second
    // chip beside the first would read as two failures.
    renderBlockResult(el, { exitCode: 1 });
    renderBlockResult(el, { exitCode: 2 });
    expect(el.querySelectorAll(".arlen-block-exit")).toHaveLength(1);
    expect(el.textContent).toBe("exit 2");

    renderBlockResult(el, { exitCode: 0 });
    expect(el.textContent).toBe("");
  });

  it("ignores the duration and the rerun hook it still accepts", () => {
    // Both are on the signature while the block menu takes them over; neither
    // may put anything on screen in the meantime.
    renderBlockResult(el, { exitCode: 0, durationMs: 4200, onRerun: () => {} });
    expect(el.children).toHaveLength(0);
  });
});
