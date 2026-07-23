/// The Knowledge app's browser adapter: the one seam between the shared kit browser
/// controller and this host's scoped KG reads. Parallel to the Files app's
/// `adapter.ts`, but every location is a virtual graph place, never a filesystem
/// path.
///
/// Mock-vs-live: fixture-backed. The scoped read commands the live app rides
/// (`typed_read`, `valid_as_of`, `retrieval`, `knowledge_stats_get`, the
/// `~/.timeline` FUSE listing, the scoped provenance read) are coder seams not
/// exposed to this app yet, so a single `knowledge_list(location)` intent stands in
/// and, failing under vite, the store serves a fixture per place so the shell
/// navigates + renders.
import { invoke } from "@tauri-apps/api/core";
import { writable } from "svelte/store";
import type { BrowserAdapter, FileEntry } from "@arlen/ui-kit/components/browser";

/// True while the content is the FIXTURE rather than this machine's real graph, so
/// the surface can say so and never pass invented activity as recorded history.
export const mocked = writable(false);

const now = Math.floor(Date.now() / 1000);
const ago = (seconds: number): number => now - seconds;

function node(name: string, when: number, kind: FileEntry["kind"] = "file"): FileEntry {
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

/// One fixture listing per place. Rich enough that the dense-context value of the
/// graph reads, not a thin activity gimmick.
const FIXTURES: Record<string, FileEntry[]> = {
  timeline: [
    node("Opened Quarterly report.pdf", ago(60 * 8)),
    node("Focus - Terminal, ~/Repositories/arlen", ago(60 * 22)),
    node("Ran cargo build in arlen", ago(60 * 34)),
    node("Agent tagged 3 files to Thesis", ago(60 * 92)),
    node("Edited chapter-3.md", ago(60 * 140)),
    node("Session - Website redesign", ago(60 * 60 * 5)),
    node("Imported 12 papers from Zotero", ago(60 * 60 * 26)),
  ],
  projects: [
    node("Arlen OS", ago(60 * 34)),
    node("Thesis", ago(60 * 92)),
    node("Website redesign", ago(60 * 60 * 5)),
    node("Reading - transformers", ago(60 * 60 * 26)),
  ],
  searches: [
    node("Touched by cargo build", ago(60 * 34)),
    node("Related to Thesis", ago(60 * 92)),
    node("Opened this week", ago(60 * 60 * 20)),
    node("Papers I have not read", ago(60 * 60 * 26)),
  ],
  library: [
    node("Attention Is All You Need - paper", ago(60 * 60 * 26)),
    node("The Rust Programming Language - book", ago(60 * 60 * 40)),
    node("Re: review notes - mail thread", ago(60 * 60 * 3)),
    node("Deep work - reading note", ago(60 * 60 * 50)),
  ],
  capsules: [node("Thesis slice - expires in 5 days", ago(60 * 60 * 18))],
};

/// The Knowledge browser adapter. Live: routes each place to its scoped read;
/// under vite, serves the fixture.
export const knowledgeAdapter: BrowserAdapter = {
  list: async (location: string): Promise<FileEntry[]> => {
    try {
      const entries = await invoke<FileEntry[]>("knowledge_list", { location });
      mocked.set(false);
      return entries;
    } catch {
      mocked.set(true);
      return FIXTURES[location] ?? [];
    }
  },
};
