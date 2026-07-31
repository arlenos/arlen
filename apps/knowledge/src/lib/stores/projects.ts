/// The Projects browser model (knowledge-app.md KA-R3): a hierarchical
/// adapter over virtual slash paths so the kit's Miller columns render the
/// relationship drill-down exactly as designed - project, then its members
/// (FILE_PART_OF), then one member's relationship hops. Never a graph view.
///
/// As-of (`valid_as_of`, direction 1): `asOf` holds the unix time the columns
/// answer for; null is now. Live both ride one scoped read,
/// `knowledge_projects_list(path, asOf)` - a coder seam; under vite the
/// fixture stands in, with a genuinely different past state so the time
/// travel shows real change, and `projectsMocked` says so.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { BrowserAdapter, FileEntry } from "@arlen/ui-kit/components/browser";
import { type TimelineEvent } from "$lib/stores/timeline";

/// True while the columns are the FIXTURE rather than the real graph.
export const projectsMocked = writable(false);

/// The moment the columns answer for; null is now. Unix seconds.
export const asOf = writable<number | null>(null);

const now = Math.floor(Date.now() / 1000);
const daysAgo = (d: number): number => now - d * 86400;

function entry(name: string, kind: FileEntry["kind"], when: number): FileEntry {
  return {
    name,
    kind,
    size: null,
    modified_unix: when,
    is_hidden: false,
    readonly: true,
    symlink_target: null,
  };
}

interface ProjectFixture {
  name: string;
  detected: number;
  /// Member name -> its relationship leaves.
  members: Record<string, string[]>;
  /// The state further back, when it differs; the as-of swap must show real
  /// change, not the same list twice.
  pastMembers?: Record<string, string[]>;
}

const PROJECTS: ProjectFixture[] = [
  {
    name: "Arlen OS",
    detected: daysAgo(210),
    members: {
      "compositor.toml": ["edited in Text editor", "in session Arlen OS", "captured from activity"],
      "design-system.md": ["opened in Files", "captured from activity"],
      "justfile": ["ran just dev in Terminal", "captured from activity"],
    },
    pastMembers: {
      "compositor.toml": ["edited in Text editor", "captured from activity"],
    },
  },
  {
    name: "Thesis",
    detected: daysAgo(90),
    members: {
      "chapter-3.md": ["edited in Text editor", "tagged by the assistant", "captured from activity"],
      "Attention Is All You Need.pdf": ["imported by the Zotero bridge", "opened in Files"],
      "notes.md": ["edited in Text editor", "captured from activity"],
    },
    pastMembers: {
      "chapter-3.md": ["edited in Text editor", "captured from activity"],
      "notes.md": ["edited in Text editor", "captured from activity"],
    },
  },
  {
    name: "Website redesign",
    detected: daysAgo(12),
    members: {
      "landing.fig": ["opened in Files", "in session Website redesign"],
      "hero.css": ["edited in Text editor", "in session Website redesign"],
    },
  },
  {
    name: "Reading - transformers",
    detected: daysAgo(30),
    members: {
      "Attention Is All You Need.pdf": ["imported by the Zotero bridge"],
    },
  },
];

/// A project that did not exist at the as-of moment is absent from the past
/// listing entirely (it accumulated later).
function fixtureList(path: string, at: number | null): FileEntry[] {
  const past = at !== null;
  const segs = path.split("/").filter(Boolean).slice(1);
  if (segs.length === 0) {
    return PROJECTS.filter((p) => !past || p.detected <= (at as number)).map((p) =>
      entry(p.name, "dir", p.detected)
    );
  }
  const project = PROJECTS.find((p) => p.name === segs[0]);
  if (!project) return [];
  const members = past && project.pastMembers ? project.pastMembers : project.members;
  if (segs.length === 1) {
    return Object.keys(members).map((m) => entry(m, "dir", project.detected));
  }
  const leaves = members[segs[1]] ?? [];
  return leaves.map((l) => entry(l, "file", project.detected));
}

let currentAsOf: number | null = null;
asOf.subscribe((v) => (currentAsOf = v));

/// The hierarchical adapter the Miller columns ride. Live: one scoped read per
/// level; fixture under vite.
export const projectsAdapter: BrowserAdapter = {
  list: async (location: string): Promise<FileEntry[]> => {
    try {
      const entries = await invoke<FileEntry[]>("knowledge_projects_list", {
        path: location,
        asOf: currentAsOf,
      });
      projectsMocked.set(false);
      return entries;
    } catch {
      projectsMocked.set(true);
      return fixtureList(location, currentAsOf);
    }
  },
};

/// The project-level facts for the detail panel: how it was detected, how many
/// members it carries, and its recent recorded activity (the timeline rows
/// filtered to this project - one anatomy, two surfaces).
export interface ProjectInfo {
  name: string;
  memberCount: number;
  detected: number;
  events: TimelineEvent[];
}

/// Info for a selected project, or null when the name is unknown. Takes the
/// timeline's flat events so the two fixtures stay in step.
export function projectInfo(name: string, events: TimelineEvent[], at: number | null): ProjectInfo | null {
  const p = PROJECTS.find((x) => x.name === name);
  if (!p) return null;
  const members = at !== null && p.pastMembers ? p.pastMembers : p.members;
  return {
    name,
    memberCount: Object.keys(members).length,
    detected: p.detected,
    events: events.filter((e) => e.project === name && (at === null || e.at <= at)).slice(0, 5),
  };
}

/// The candidate as-of days the picker offers (the fixture's horizon); live
/// this comes from the graph's recorded range.
export function asOfCandidates(): number[] {
  return [1, 2, 3, 5, 7, 14].map((d) => {
    const dd = new Date((now - d * 86400) * 1000);
    dd.setHours(18, 0, 0, 0);
    return Math.floor(dd.getTime() / 1000);
  });
}
