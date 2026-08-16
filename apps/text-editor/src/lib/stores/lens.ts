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
  members: ProjectMember[];
}

/// A sibling file. `name` is what the chip shows, `path` is what opens - a
/// basename is not openable, and two projects can hold the same one.
export interface ProjectMember {
  path: string;
  name: string;
}

interface LensState {
  provenance: ProvenanceStep[];
  related: Backlink[];
  project: ProjectContext | null;
  /// The RELATED section alone is still a sample; see `loadLens`.
  relatedMocked: boolean;
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
  project: {
    name: "Arlen editor",
    members: [
      { path: "/example/arlen-editor/roadmap.md", name: "roadmap.md" },
      { path: "/example/arlen-editor/provenance.md", name: "provenance.md" },
      { path: "/example/arlen-editor/lens-design.md", name: "lens-design.md" },
    ],
  },
};

// `mocked: true` because this IS the fixture. The panel renders before (and
// without) any `loadLens`, so flagging the initial value as live claimed invented
// provenance, backlinks and a project as the open file's real graph neighbourhood.
export const lens = writable<LensState>({ ...FIXTURE, mocked: true, relatedMocked: true });

/// Load the lens for a file. Live: the three graph queries; fixture under vite.
export async function loadLens(ref: string): Promise<void> {
  try {
    // Provenance AND project are asked for; `related_of` still is not, and that
    // is a missing MEANING rather than a missing permission - the graph holds no
    // file-to-file edge, so "backlinks" has nothing to traverse yet (see
    // `lens.rs`). The old note here blamed the read gate for the other two: wrong
    // on both counts, since these commands live in THIS app now and the gate
    // authorises a membership traversal by its endpoints (measured 16 August).
    //
    // Asked SEPARATELY so one absent answer cannot cost the other. Failing the
    // whole load is what used to drop the entire panel to its fixture, including
    // the part that was real.
    const provenance = await invoke<ProvenanceStep[]>("provenance_of", { ref });
    // Backlinks, asked separately for the same reason as the project: one absent
    // answer must not cost the others.
    let related = FIXTURE.related;
    let relatedMocked = true;
    try {
      related = await invoke<Backlink[]>("related_of", { ref });
      relatedMocked = false;
    } catch {
      // Keep the labelled sample for this section only.
    }
    let project: ProjectContext | null = FIXTURE.project;
    let projectMocked = true;
    try {
      const ctx = await invoke<ProjectContext | null>("project_of", { ref });
      // A file in no project is NULL, not a project with an empty name. The panel
      // guards the section with `{#if $lens.project}` and an object is truthy, so
      // the empty-name version rendered "Part of" followed by nothing - a claim
      // about a project that does not exist, which is the failure this whole
      // section was rebuilt to stop making.
      // The siblings ride the same query now (plan #4), so the section names the
      // project AND opens into it rather than showing a heading over nothing.
      project = ctx && ctx.name ? ctx : null;
      projectMocked = false;
    } catch {
      // Keep the labelled sample for this section only.
    }
    // `mocked` is the WHOLE-panel claim and is now false, because provenance and
    // project are real. `relatedMocked` carries the one section that is still a
    // sample - dropping the global caption without it made invented backlinks
    // read as this file's real neighbourhood, which is the exact thing the
    // caption existed to prevent.
    lens.set({
      provenance,
      related,
      project,
      mocked: projectMocked,
      relatedMocked,
    });
  } catch {
    lens.set({ ...FIXTURE, mocked: true, relatedMocked: true });
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
