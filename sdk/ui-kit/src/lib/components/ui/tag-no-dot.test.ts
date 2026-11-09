// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: MIT

// @vitest-environment jsdom
/// No round dot inside a tag. Tim, 9 November 2026, about the tags on the
/// website and in halbat: "dass da immer ein runder punkt innerhalb des tags
/// ist (links innerhalb der box) das mag ich glaub raus ... will ich überall
/// raus". The website drew it as a `::before` on its pill and as a span in its
/// filter chips; both are gone there. The kit's Badge and Chip never had one,
/// and this keeps it that way: a chip without `lead` starts with its label, and
/// neither component's source draws a round pseudo-element.
///
/// What this does not look at: an app passing a dot in through `lead`. That is
/// the caller's choice and shows in the app, not here.
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { render } from "@testing-library/svelte";
import Chip from "./chip/chip.svelte";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");

/// A `::before` or `::after` rule that is round: the shape of the website's dot.
function drawsRoundPseudo(src: string): boolean {
  const rules = src.match(/::?(before|after)\s*\{[^}]*\}/g) ?? [];
  return rules.some((r) => /border-radius:\s*(50%|999|9999|var\(--radius-(round|full|pill))/.test(r) || /rounded-full/.test(r));
}

describe("tags carry no dot", () => {
  it("the check sees the dot the website had, and passes a plain pill", () => {
    const dot = `.pille::before { content:""; width:6px; height:6px; border-radius:50%; background: var(--f); }`;
    const plain = `.pille { display:inline-flex; border-radius: var(--r-ctl); }`;
    expect(drawsRoundPseudo(dot)).toBe(true);
    expect(drawsRoundPseudo(plain)).toBe(false);
  });

  it("Chip and Badge draw no round pseudo-element", () => {
    for (const f of ["./chip/chip.svelte", "./badge/badge.svelte"]) {
      const src = read(f);
      expect(src.length, `${f} was not read`).toBeGreaterThan(100);
      expect(drawsRoundPseudo(src), f).toBe(false);
    }
  });

  it("a chip without lead starts with its label", () => {
    const { container } = render(Chip, { label: "Documents" });
    const body = container.querySelector(".chip-body")!;
    expect(body.firstElementChild?.classList.contains("chip-label")).toBe(true);
  });
});
