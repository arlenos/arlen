// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The history search's failure behaviour, which is the whole reason the store has
// an `unavailable` flag beside an empty result: a backend that did not answer and
// a search that found nothing look identical in a list, and the palette says two
// different sentences about them. A refactor that flattened the catch would make
// the palette tell a person their history is empty.
import { describe, it, expect, beforeEach, vi } from "vitest";
import { get } from "svelte/store";

const search = vi.fn();
vi.mock("$lib/contract", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("$lib/contract");
  return { ...actual, terminalHistorySearch: (...a: unknown[]) => search(...a) };
});

import {
  historyQuery,
  historyResults,
  historyLoaded,
  historyUnavailable,
  historyOnlyFailures,
  historyAgentOnly,
  historyProjectId,
  runHistorySearch,
} from "./history.js";

beforeEach(() => {
  search.mockReset();
  historyQuery.set("");
  historyResults.set([]);
  historyLoaded.set(false);
  historyUnavailable.set(false);
  historyOnlyFailures.set(false);
  historyAgentOnly.set(false);
  historyProjectId.set(null);
});

describe("runHistorySearch", () => {
  it("keeps what came back and says the backend answered", async () => {
    search.mockResolvedValue([{ id: "b1", command: "ls" }]);
    await runHistorySearch();
    expect(get(historyResults)).toHaveLength(1);
    expect(get(historyUnavailable)).toBe(false);
    expect(get(historyLoaded)).toBe(true);
  });

  it("separates a backend that refused from a search that found nothing", async () => {
    search.mockRejectedValue(new Error("no socket"));
    await runHistorySearch();
    expect(get(historyResults)).toEqual([]);
    expect(get(historyUnavailable)).toBe(true);
    // Loaded either way: the palette has an answer to render, and which one it
    // renders is exactly what the flag decides.
    expect(get(historyLoaded)).toBe(true);
  });

  it("clears the refusal once a later search answers", async () => {
    search.mockRejectedValue(new Error("no socket"));
    await runHistorySearch();
    expect(get(historyUnavailable)).toBe(true);

    search.mockReset();
    search.mockResolvedValue([]);
    await runHistorySearch();
    // Empty AND available: found nothing, which is the other sentence.
    expect(get(historyResults)).toEqual([]);
    expect(get(historyUnavailable)).toBe(false);
  });

  it("sends the chips as filters, not as part of the query", async () => {
    search.mockResolvedValue([]);
    historyQuery.set("cargo");
    historyOnlyFailures.set(true);
    historyAgentOnly.set(true);
    historyProjectId.set("p1");
    await runHistorySearch();

    const [query, filters] = search.mock.calls[0] as [string, Record<string, unknown>];
    expect(query).toBe("cargo");
    expect(filters.only_failures).toBe(true);
    expect(filters.origin).toBe("agent");
    expect(filters.project_id).toBe("p1");
  });

  it("asks for every origin when the agent chip is off", async () => {
    search.mockResolvedValue([]);
    await runHistorySearch();
    const [, filters] = search.mock.calls[0] as [string, Record<string, unknown>];
    expect(filters.origin).toBe(null);
  });
});
