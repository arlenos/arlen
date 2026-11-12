/// The review list for identity suggestions (bitemporal-knowledge-graph.md
/// §4.11 and §4.12): the graph proposes that two entries are the same person,
/// place or thing, and only the person decides.
///
/// Nodes are never merged. "Same" is a `must_link`, "Different" is a
/// `cannot_link`, and both are commands that can be taken back: taking one back
/// closes an edge and loses nothing. "Not sure" writes nothing at all.
///
/// **This store runs on its fixture only.** The daemon records the decisions
/// and nothing reads them out yet; the host commands are the coder's, after
/// his item 13. Until then the place is offered under vite and nowhere else,
/// so no real window shows invented people.
import { derived, writable } from "svelte/store";

/// What kind of thing an entry is. Sets the icon and nothing else.
export type EntryKind = "person" | "place" | "thing";

/// Why the graph thinks two entries match. A closed set, so the sentence is the
/// catalogue's and reaches every reader in their own language. Ordered from the
/// strongest evidence to the weakest; the list is sorted by it and never prints
/// a score.
export type MatchReason = "email" | "phone" | "name" | "together";
const STRENGTH: MatchReason[] = ["email", "phone", "name", "together"];

/// One entry as the list shows it: what it is called and the one line that
/// tells it apart from its neighbour.
export interface Entry {
  id: string;
  name: string;
  kind: EntryKind;
  /// The detail a person recognises it by: an address, a number, a place.
  detail: string;
  /// Where the graph knows it from, in the source's own words.
  source: string;
}

/// What the person decided about one candidate, or nothing yet.
export type Decision = "same" | "different" | null;

/// One possible match for an entry.
export interface Candidate extends Entry {
  reason: MatchReason;
  decision: Decision;
}

/// One entry with everything the graph thinks might be the same thing. A group
/// and not a pair: three spellings of one person are one question about one
/// person, not three questions.
export interface Suggestion {
  entry: Entry;
  candidates: Candidate[];
}

// i18n-foreign: names, addresses and document titles from a person's own
// graph. Content, not interface.
const FIXTURE: Suggestion[] = [
  {
    entry: { id: "e-anna", name: "Anna Moser", kind: "person", detail: "anna.moser@uni.example", source: "Contacts" },
    candidates: [
      { id: "c-anna-1", name: "Anna M.", kind: "person", detail: "anna.moser@uni.example", source: "Mail, 3 May", reason: "email", decision: null },
      { id: "c-anna-2", name: "A. Moser", kind: "person", detail: "Thesis outline", source: "Notes, 12 April", reason: "name", decision: null },
    ],
  },
  {
    entry: { id: "e-jonas", name: "Jonas Berger", kind: "person", detail: "+43 660 555 0142", source: "Contacts" },
    candidates: [
      { id: "c-jonas-1", name: "Jonas", kind: "person", detail: "+43 660 555 0142", source: "Calendar, 9 June", reason: "phone", decision: null },
    ],
  },
  {
    entry: { id: "e-lab", name: "Institute of Computer Science", kind: "place", detail: "Technikerstraße 21a", source: "Calendar" },
    candidates: [
      { id: "c-lab-1", name: "ICS seminar room", kind: "place", detail: "Reading group", source: "Notes, 2 March", reason: "together", decision: null },
    ],
  },
];

/// A long list for the render loop: the same three shapes, numbered, so the
/// surface can be looked at with three hundred open questions.
function many(n: number): Suggestion[] {
  const out: Suggestion[] = [];
  for (let i = 0; i < n; i += 1) {
    const base = FIXTURE[i % FIXTURE.length];
    out.push({
      entry: { ...base.entry, id: `${base.entry.id}-${i}`, name: i < FIXTURE.length ? base.entry.name : `${base.entry.name} ${i + 1}` },
      candidates: base.candidates.map((c) => ({ ...c, id: `${c.id}-${i}` })),
    });
  }
  return out;
}

/// Strongest evidence first, by each group's best candidate.
export function byStrength(list: Suggestion[]): Suggestion[] {
  const best = (s: Suggestion): number =>
    Math.min(...s.candidates.map((c) => STRENGTH.indexOf(c.reason)));
  return [...list].sort((a, b) => best(a) - best(b));
}

/// Every suggestion, decided or not.
export const suggestions = writable<Suggestion[]>([]);

/// How many candidates still wait for an answer. The sidebar shows it and the
/// row is absent at zero with nothing decided either.
export const openCount = derived(suggestions, ($s) =>
  $s.reduce((n, g) => n + g.candidates.filter((c) => c.decision === null).length, 0),
);

/// How many candidates carry a decision that can be taken back.
export const decidedCount = derived(suggestions, ($s) =>
  $s.reduce((n, g) => n + g.candidates.filter((c) => c.decision !== null).length, 0),
);

/// Load the list. `shape` picks the fixture the render loop asks for.
export function loadReview(shape: "few" | "none" | "many" = "few"): void {
  const list = shape === "none" ? [] : shape === "many" ? many(300) : FIXTURE;
  suggestions.set(byStrength(structuredClone(list)));
}

/// Record a decision about one candidate, or take it back with `null`.
export function decide(candidateId: string, decision: Decision): void {
  suggestions.update((list) =>
    list.map((g) => ({
      ...g,
      candidates: g.candidates.map((c) => (c.id === candidateId ? { ...c, decision } : c)),
    })),
  );
}
