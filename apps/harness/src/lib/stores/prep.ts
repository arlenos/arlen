/// The prep-for-this surface (agent-work-surfaces-plan.md surface 3): pull all
/// live KG context on a chosen entity, on demand - the cleanest "the KG earns
/// its keep" demo. Pure read, no gate.
///
/// Mock-vs-live: live via `working_set_briefing` (the pick list of your live
/// entities) + `prep_for` (the prepped context for a chosen entity); a fixture
/// stands in under vite so the surface is verifiable without the graph daemon.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// One related entity in a prep result (os-sdk `graph::PrepItem`).
export interface PrepItem {
  /// The entity's node id.
  id: string;
  /// A human label (path basename / project name / meeting title).
  label: string;
  /// The KG node type (File / Project / Meeting / Person / ...).
  kind: string;
  /// How it relates to the subject (a rendered edge phrase).
  relation: string;
  /// The liveness bucket the surface groups on.
  liveness: "live" | "dormant" | "stale";
  /// The liveness score in [0, 1] - the sort key (higher = more live).
  score: number;
}

// The user's live working set - the pick list. Live: `working_set_briefing`.
const WORKING_SET_FIXTURE: PrepItem[] = [
  { id: "mtg-tim-2pm", label: "2pm with Tim", kind: "Meeting", relation: "upcoming", liveness: "live", score: 0.94 },
  { id: "proj-arlen", label: "Arlen OS", kind: "Project", relation: "active project", liveness: "live", score: 0.9 },
  { id: "file-consent-plan", label: "system-dialog-plan.md", kind: "File", relation: "edited today", liveness: "live", score: 0.83 },
  { id: "person-tim", label: "Tim", kind: "Person", relation: "collaborator", liveness: "live", score: 0.79 },
  { id: "proj-harness", label: "AI harness", kind: "Project", relation: "worked on this week", liveness: "dormant", score: 0.55 },
];

// The prepped context per subject. Live: `prep_for(subjectId)`. Items arrive
// score-ranked from the backend; the surface groups them by liveness.
const PREP_FIXTURE: Record<string, PrepItem[]> = {
  "mtg-tim-2pm": [
    { id: "proj-arlen", label: "Arlen OS", kind: "Project", relation: "what the meeting is about", liveness: "live", score: 0.95 },
    { id: "file-consent-plan", label: "system-dialog-plan.md", kind: "File", relation: "you both edited", liveness: "live", score: 0.9 },
    { id: "note-prep-direction", label: "Prep-for-this direction", kind: "Note", relation: "last meeting's note", liveness: "live", score: 0.84 },
    { id: "file-agent-surfaces", label: "agent-work-surfaces-plan.md", kind: "File", relation: "cited last time", liveness: "live", score: 0.78 },
    { id: "file-capsule", label: "capsule-mint-plan.md", kind: "File", relation: "co-accessed last week", liveness: "dormant", score: 0.48 },
    { id: "proj-lunaris", label: "Lunaris (pre-rename)", kind: "Project", relation: "linked, untouched for weeks", liveness: "stale", score: 0.19 },
  ],
  "proj-arlen": [
    { id: "mtg-tim-2pm", label: "2pm with Tim", kind: "Meeting", relation: "next about this", liveness: "live", score: 0.9 },
    { id: "file-consent-plan", label: "system-dialog-plan.md", kind: "File", relation: "part of the project", liveness: "live", score: 0.86 },
    { id: "file-roadmap", label: "arlen-roadmap.md", kind: "File", relation: "part of the project", liveness: "live", score: 0.8 },
    { id: "person-tim", label: "Tim", kind: "Person", relation: "works here", liveness: "live", score: 0.75 },
    { id: "file-old-distro", label: "distro/build.sh (retired)", kind: "File", relation: "part of, superseded", liveness: "stale", score: 0.15 },
  ],
};

/// The pick list of the user's live entities.
export const workingSet = writable<PrepItem[]>([]);
/// The entity currently being prepped for, or null.
export const subject = writable<PrepItem | null>(null);
/// The prepped context for the current subject.
export const prepped = writable<PrepItem[]>([]);
/// True while a prep gather is in flight.
export const loading = writable(false);

/// Load the pick list. Live: `working_set_briefing`; fixture under vite.
export async function loadWorkingSet(): Promise<void> {
  try {
    workingSet.set(await invoke<PrepItem[]>("working_set_briefing", { limit: 24 }));
  } catch {
    workingSet.set(WORKING_SET_FIXTURE);
  }
}

/// Prep for an entity. Live: `prep_for`; fixture under vite.
export async function prepFor(item: PrepItem): Promise<void> {
  subject.set(item);
  loading.set(true);
  try {
    prepped.set(await invoke<PrepItem[]>("prep_for", { subjectId: item.id, limit: 40 }));
  } catch {
    prepped.set(PREP_FIXTURE[item.id] ?? []);
  }
  loading.set(false);
}

/// Clear the subject, back to the pick list.
export function clearSubject(): void {
  subject.set(null);
  prepped.set([]);
}
