/// "Ask Arlen": the natural-language front-end to the faceted filter. A question
/// scoped to the current folder is sent to the assistant, which drafts a facet
/// query; the draft populates the live facet selection (so the user SEES the
/// parsed query as editable chips and the listing as a preview), under a banner
/// that names what was read. Pull, never push: nothing is saved or moved until
/// the user acts. When the assistant is off, the Ask mode is unavailable.
///
/// The scoped-ask command does not exist yet (coder seam, `files_ask`); until it
/// lands the surface drives the review against mocked drafts, and the off-switch
/// read (`files_ai_enabled`) defaults the Ask mode to unavailable.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { selectedFacets, facetOpen, FACET_GROUPS, type FacetGroup } from "./facets";

/// The search row's mode: a literal name search, or a natural-language ask.
export type AskMode = "search" | "ask";
export const askMode = writable<AskMode>("search");

/// True while the ask is in flight.
export const askLoading = writable(false);

/// What the assistant read to draft the current filter, for the transparency
/// line (the anti-Recall move: the reads are shown, the audit is the guarantee).
export interface AskReads {
  files: number;
  tags: number;
}

/// The active draft: the question that produced the current facets, and the
/// reads. null = no draft (the facet bar is manual).
export interface AskDraft {
  query: string;
  reads: AskReads;
}
export const askDraft = writable<AskDraft | null>(null);

/// Whether the assistant is enabled (the off-switch). When false the Ask mode is
/// greyed and the bar stays literal search.
export const aiEnabled = writable(false);

/// The shape `files_ask` returns: a drafted facet selection in the existing
/// vocabulary, plus what it read.
export interface AskResult {
  facets: Partial<Record<FacetGroup, string[]>>;
  reads: AskReads;
}

/// Read whether the assistant is on, so the Ask mode can grey out. A missing
/// command (no AI backend) leaves Ask unavailable rather than half-wired.
export async function loadAiEnabled(): Promise<void> {
  try {
    aiEnabled.set(await invoke<boolean>("files_ai_enabled"));
  } catch {
    aiEnabled.set(false);
  }
}

/// Send a scoped natural-language ask; returns the drafted result, or null on
/// failure (the caller shows the no-draft fallback).
export async function runAsk(folder: string, query: string): Promise<AskResult | null> {
  askLoading.set(true);
  try {
    return await invoke<AskResult>("files_ask", { folder, query });
  } catch {
    return null;
  } finally {
    askLoading.set(false);
  }
}

/// Adopt a drafted facet set into the live facet selection + the banner, and
/// reveal the facet bar so the chips show. The caller navigates to the result.
export function applyDraft(result: AskResult, query: string): void {
  const sel: Record<FacetGroup, Set<string>> = {
    project: new Set(),
    type: new Set(),
    time: new Set(),
    touched: new Set(),
  };
  for (const g of FACET_GROUPS) for (const v of result.facets[g] ?? []) sel[g].add(v);
  selectedFacets.set(sel);
  askDraft.set({ query, reads: result.reads });
  facetOpen.set(true);
}

/// Drop the active draft (the chips stay; only the banner clears).
export function clearAsk(): void {
  askDraft.set(null);
}
