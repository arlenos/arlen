/// "Ask Arlen": the natural-language front-end to the faceted filter. A question
/// scoped to the current folder is sent to the assistant, which drafts a facet
/// query; the draft populates the live facet selection (so the user SEES the
/// parsed query as editable chips and the listing as a preview), under a banner
/// that names what was read. Pull, never push: nothing is saved or moved until
/// the user acts. When the assistant is off, the Ask mode is unavailable.
///
/// `files_ask` is implemented and this store calls it. The comment here said the
/// command "does not exist yet (coder seam)" and that the surface "drives the
/// review against mocked drafts" - both halves false, and no mock is left in this
/// file to have driven anything. A note about a MISSING neighbour has a shelf
/// life the code does not, and this one outlived its subject.

import { derived, writable } from "svelte/store";
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

/// What the AI is allowed to do right now, as `ai_capability` reports it - the
/// same command the harness and the shell read, rather than a fourth narrower
/// mirror. null means the read itself failed: the backend is not answering, which
/// is a different fact from the AI being switched off and is said differently.
export interface AskCapability {
  enabled: boolean;
  tier: string;
  actionMode: string;
  provider: string | null;
  model: string | null;
  executorLive: boolean;
}
export const askCapability = writable<AskCapability | null>(null);

/// Whether the capability has been read at all. Until it has, the bar shows no
/// claim either way - an app that has not looked yet must not say the AI is off.
export const askCapabilityLoaded = writable(false);

/// Whether the assistant is enabled (the off-switch). Derived rather than stored,
/// so there is one source for the posture and the boolean cannot drift from the
/// sentence shown beside it.
export const aiEnabled = derived(askCapability, (c) => c?.enabled ?? false);

/// The shape `files_ask` returns: a drafted facet selection in the existing
/// vocabulary, plus what it read.
export interface AskResult {
  facets: Partial<Record<FacetGroup, string[]>>;
  reads: AskReads;
}

/// Read the capability behind the Ask affordance. A failed read is kept as null
/// rather than flattened to "off": those are two different things for the person
/// in front of it, one to change in Settings and one to wait out.
export async function loadAskCapability(): Promise<void> {
  try {
    askCapability.set(await invoke<AskCapability>("ai_capability"));
  } catch {
    askCapability.set(null);
  } finally {
    askCapabilityLoaded.set(true);
  }
}

/// Which message the line under the search bar carries, for the two states that
/// speak; the healthy state renders nothing, which is the shell's rule and Tim's -
/// a status line that is always on is the noise the rule exists to prevent.
///
/// Returns a catalogue key rather than a sentence: the shell hands back English
/// because it has no catalogue, and copying that here would put untranslatable
/// text in a translated app. Same vocabulary, said about the thing this surface
/// offers - not the agent in general, but asking about these files.
export function askCapabilityMessage(c: AskCapability | null): string {
  return c === null ? "f.ask.unreachable" : "f.ask.aiOff";
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
