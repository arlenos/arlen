/// The lineage model (knowledge-app.md §3.6, KA-R7): "where did this come
/// from, what touched it, why is it here" as a SENTENCE LIST - aggregation
/// plus degree-of-interest, never a node-link diagram. Every hop renders a
/// reason and a relationship in plain words (the comprehension metric), never
/// a raw identifier. Live: `knowledge_provenance` is the scoped provenance
/// read (a coder seam over the FILE_PART_OF provenance columns + the bridge
/// origin tags); under vite a fixture map covers the story's known nodes, and
/// an unknown node gets ONLY its origin line - honestly thin, never invented
/// rich.
import { invoke } from "@tauri-apps/api/core";

/// One lineage hop, already phrased: quiet verb, emphasized subject, moment.
export interface ProvenanceHop {
  verb: string;
  subject: string;
  when?: number;
}

const now = Math.floor(Date.now() / 1000);
const daysAgo = (d: number, h = 12): number => {
  const dd = new Date(now * 1000);
  dd.setHours(h, 0, 0, 0);
  return Math.floor(dd.getTime() / 1000) - d * 86400;
};

const FIXTURE: Record<string, ProvenanceHop[]> = {
  "compositor.toml": [
    { verb: "created in", subject: "Text editor", when: daysAgo(160, 10) },
    { verb: "part of", subject: "Arlen OS", when: daysAgo(160, 10) },
    { verb: "edited in", subject: "Text editor", when: daysAgo(2, 10) },
    { verb: "read by", subject: "just dev", when: daysAgo(2, 10) },
    { verb: "in session", subject: "Arlen OS", when: daysAgo(2, 12) },
  ],
  "chapter-3.md": [
    { verb: "created in", subject: "Text editor", when: daysAgo(40, 15) },
    { verb: "part of", subject: "Thesis", when: daysAgo(40, 15) },
    { verb: "tagged by", subject: "the assistant", when: daysAgo(0, 10) },
    { verb: "edited in", subject: "Text editor", when: daysAgo(1, 11) },
  ],
  "Attention Is All You Need": [
    { verb: "imported by", subject: "the Zotero bridge", when: daysAgo(1, 13) },
    { verb: "part of", subject: "Thesis", when: daysAgo(1, 13) },
    { verb: "opened in", subject: "Files", when: daysAgo(1, 13) },
  ],
  "Re: review notes": [
    { verb: "imported by", subject: "the Thunderbird bridge", when: daysAgo(3, 8) },
    { verb: "from", subject: "alex@example.com", when: daysAgo(3, 8) },
  ],
  "hero.css": [
    { verb: "created in", subject: "Text editor", when: daysAgo(11, 14) },
    { verb: "part of", subject: "Website redesign", when: daysAgo(11, 14) },
    { verb: "edited in", subject: "Text editor", when: daysAgo(0, 16) },
    { verb: "in session", subject: "Website redesign", when: daysAgo(0, 16) },
  ],
  "The Rust Programming Language": [
    { verb: "imported by", subject: "the Calibre bridge", when: daysAgo(20, 19) },
    { verb: "opened in", subject: "Library", when: daysAgo(3, 21) },
  ],
};

// Papers share the pdf suffix with their library titles; normalize the
// lookup so the timeline's "Attention Is All You Need.pdf" hits the same
// lineage as the library's plain title.
function keyFor(name: string): string {
  return name.replace(/\.pdf$/, "");
}

/// The lineage for a node. An unknown node answers with only its origin -
/// the one thing the graph always knows.
export async function provenanceFor(name: string): Promise<ProvenanceHop[]> {
  try {
    return await invoke<ProvenanceHop[]>("knowledge_provenance", { node: name });
  } catch {
    return FIXTURE[keyFor(name)] ?? [{ verb: "origin", subject: "captured from activity" }];
  }
}
