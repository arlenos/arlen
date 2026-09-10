// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Adopting a drafted ask into the live filter, and the one sentence the surface
// says when it cannot offer the ask at all. The draft is the pull-not-push
// promise in code: the assistant's answer becomes editable chips and a banner
// naming what it read, and nothing else happens - so what it writes into the
// selection, and what it leaves alone, is the whole contract.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { get } from "svelte/store";

// The store dials `ai_capability` and `files_ask` on import paths this test never
// takes; mocked so nothing reaches for a Tauri that is not here.
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => ({}) }));

import { selectedFacets, facetOpen } from "./facets.js";
import { askDraft, askCapabilityMessage, applyDraft, clearAsk, type AskResult } from "./ask.js";

const reads = { folder: "/home/u/Bilder", count: 12 } as unknown as AskResult["reads"];

function draft(facets: AskResult["facets"]): AskResult {
  return { facets, reads };
}

describe("askCapabilityMessage", () => {
  it("says two different things about off and unreachable", () => {
    // One is a switch in Settings, the other is something to wait out. A surface
    // that flattens them tells a person to go and change a setting that is
    // already right.
    expect(askCapabilityMessage(null)).toBe("f.ask.unreachable");
    expect(askCapabilityMessage({ enabled: false } as never)).toBe("f.ask.aiOff");
  });
});

describe("applyDraft", () => {
  beforeEach(() => {
    selectedFacets.set({
      project: new Set(),
      type: new Set(),
      time: new Set(),
      touched: new Set(),
    });
    facetOpen.set(false);
    askDraft.set(null);
  });

  it("turns the drafted facets into the live selection", () => {
    applyDraft(draft({ type: ["image"], time: ["last-week"] }), "bilder von letzter woche");
    const sel = get(selectedFacets);
    expect([...sel.type]).toEqual(["image"]);
    expect([...sel.time]).toEqual(["last-week"]);
    expect([...sel.project]).toEqual([]);
  });

  it("REPLACES what was selected rather than adding to it", () => {
    // A draft is an answer to one question. Merging it into a previous selection
    // would show a listing that answers neither.
    selectedFacets.set({
      project: new Set(["p1"]),
      type: new Set(["document"]),
      time: new Set(),
      touched: new Set(),
    });
    applyDraft(draft({ type: ["image"] }), "bilder");
    const sel = get(selectedFacets);
    expect([...sel.type]).toEqual(["image"]);
    expect([...sel.project]).toEqual([]);
  });

  it("opens the facet bar, because the chips are the point", () => {
    applyDraft(draft({ type: ["image"] }), "bilder");
    expect(get(facetOpen)).toBe(true);
  });

  it("keeps the question and what was read, for the banner", () => {
    applyDraft(draft({ type: ["image"] }), "welche bilder sind neu");
    expect(get(askDraft)).toMatchObject({ query: "welche bilder sind neu", reads });
  });

  it("ignores a group the draft does not mention", () => {
    applyDraft(draft({}), "nichts davon");
    const sel = get(selectedFacets);
    expect([...sel.type]).toEqual([]);
    expect([...sel.touched]).toEqual([]);
  });
});

describe("clearAsk", () => {
  it("drops the banner and leaves the chips alone", () => {
    applyDraft(draft({ type: ["image"] }), "bilder");
    clearAsk();
    expect(get(askDraft)).toBe(null);
    expect([...get(selectedFacets).type]).toEqual(["image"]);
    expect(get(facetOpen)).toBe(true);
  });
});
