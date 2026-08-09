/// The KG-lens (text-editor-app.md): the file's Knowledge-Graph neighbourhood
/// surfaced AUTOMATICALLY beside the text - the differentiator that makes this an
/// editor worth building, not "gedit with tabs". Provenance (coarse: where it came
/// from, AI-vs-human authorship), inline contextual backlinks (a snippet of each
/// note that references or co-occurs with this file - read-only context you act on),
/// and project membership. Nothing hand-authored; the system finds the links.
///
/// Mock-vs-live: fixture-backed. `provenance_of` (the caller-scoped read op, PH-R1),
/// the backlinks/co-occurrence query, and project-membership are coder seams on the
/// graph daemon; every query is debounced/cached off the render path. Under vite the
/// store serves a fixture.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { openPath } from "$lib/stores/document";

/// Where a step's assertion came from (mirrors the Files provenance model).
export type Provenance = "user" | "graph" | "external" | "model" | "agent";
/// How confidently the actor is known - never overclaim ("a process", not "app X").
export type Fidelity = "resolved" | "pid" | "proxy";

/// One coarse lineage step in the file's provenance.
export interface ProvenanceStep {
  relation: string;
  actor: string;
  origin: Provenance;
  when: string;
  fidelity: Fidelity;
}

/// One inline contextual backlink: a snippet of a note that references this file.
export interface Backlink {
  file: string;
  ref: string;
  snippet: string;
}

/// The file's project membership + its sibling members.
export interface ProjectContext {
  name: string;
  members: string[];
}

interface LensState {
  provenance: ProvenanceStep[];
  related: Backlink[];
  project: ProjectContext | null;
  mocked: boolean;
}

const FIXTURE = {
  provenance: [
    { relation: "Started by", actor: "you", origin: "user" as Provenance, when: "3 weeks ago", fidelity: "resolved" as Fidelity },
    { relation: "A section drafted by", actor: "the assistant", origin: "agent" as Provenance, when: "yesterday", fidelity: "resolved" as Fidelity },
    { relation: "Last opened by", actor: "a process", origin: "graph" as Provenance, when: "12 minutes ago", fidelity: "pid" as Fidelity },
  ],
  related: [
    { file: "roadmap.md", ref: "roadmap", snippet: "…the editor lands after the compositor work, see the notes in this file for the lens design…" },
    { file: "meeting-2026-06-30.md", ref: "meeting-0630", snippet: "…agreed the KG-lens is the reason to build our own editor, not gedit…" },
    { file: "provenance.md", ref: "provenance", snippet: "…coarse lineage only, captured at semantic edges, never the syscall firehose…" },
  ],
  project: { name: "Arlen editor", members: ["roadmap.md", "provenance.md", "lens-design.md"] },
};

// `mocked: true` because this IS the fixture. The panel renders before (and
// without) any `loadLens`, so flagging the initial value as live claimed invented
// provenance, backlinks and a project as the open file's real graph neighbourhood.
export const lens = writable<LensState>({ ...FIXTURE, mocked: true });

/// Load the lens for a file. Live: the three graph queries; fixture under vite.
export async function loadLens(ref: string): Promise<void> {
  try {
    // Only provenance is asked for. `related_of` and `project_of` named commands
    // that live in the FILES app's binary, so those two calls could never have
    // succeeded from here - and the edges they want (backlinks, project
    // membership) are refused by the knowledge daemon's read gate to a caller
    // that is not system-anchored anyway. Asking for all three failed the whole
    // load and dropped the panel to its fixture every time, including the part
    // that could have been real.
    const provenance = await invoke<ProvenanceStep[]>("provenance_of", { ref });
    lens.set({ provenance, related: FIXTURE.related, project: FIXTURE.project, mocked: true });
  } catch {
    lens.set({ ...FIXTURE, mocked: true });
  }
}

/// Open a related file in the editor.
///
/// It used to invoke `open_file`, a command the MEETINGS app defines - a call
/// that could only ever be rejected. This app's own reader is `editor_open`, and
/// the result has to be PUT somewhere: reading a file and dropping it on the
/// floor leaves the click doing nothing, which is what the broken version did
/// and is not an improvement over it.
export async function openRelated(file: string): Promise<void> {
  await openPath(file);
}
