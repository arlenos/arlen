/// The Timeline model (knowledge-app.md KA-R2): typed events off the graph's
/// recorded activity, grouped by day, with contiguous work clustered into
/// sessions. This is recall of what the system actually captured - typed
/// events, never screenshots - so the store never invents history: live it
/// reads the `knowledge_timeline` command (a coder seam over the FUSE timeline
/// + the typed reads); under vite a fixture stands in and `mocked` says so.
import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// What kind of thing happened; drives the row's mark and verb.
export type TimelineKind = "opened" | "edited" | "ran" | "focus" | "agent" | "imported";

/// One recorded event, already phrased as a sentence: quiet verb, emphasized
/// object (the privacy page's anatomy - what matters is the user's data).
export interface TimelineEvent {
  id: string;
  kind: TimelineKind;
  /// The quiet leading verb, as a message id ("k.tl.verb.opened").
  ///
  /// An id rather than the word: it rendered verbatim, so every row on a German
  /// timeline read "opened chapter-3.md". The lint could not see it either - a
  /// lowercase single token looks like an identifier, which is exactly what this
  /// is now. Separate from `kind` because one kind can state more than one fact:
  /// an `agent` event tags today and may summarise tomorrow.
  verb: string;
  /// The emphasized object: the file, the command, the app.
  object: string;
  /// Where it happened (the app or bridge), shown quietly.
  source: string;
  /// Unix seconds.
  at: number;
  /// The project this belongs to, when the graph knows one.
  project?: string;
}

/// A reconstructed work session: contiguous activity clustered under a title.
export interface TimelineSession {
  id: string;
  title: string;
  from: number;
  to: number;
  events: TimelineEvent[];
}

/// One item in the spine: a lone event or a session block.
export type TimelineItem =
  | { kind: "event"; event: TimelineEvent }
  | { kind: "session"; session: TimelineSession };

/// One day of the spine, newest day first, items newest first.
export interface TimelineDay {
  /// Local midnight, unix seconds; the scrub anchor.
  date: number;
  items: TimelineItem[];
}

/// The loaded spine, or null before the read settles.
export const days = writable<TimelineDay[] | null>(null);
/// A pending menu-invoked action the surface must resolve (the export run, or
/// the delete confirm). The shell's app menu is where these actions live
/// (Tim, 31 Jul); the surface only renders their consequences.
export const pendingMenuAction = writable<"export" | "deleteToday" | "deleteAll" | null>(null);
/// True while the spine is the FIXTURE, so the surface says so and never
/// passes invented activity as recorded history.
export const timelineMocked = writable(false);

/// True when a real session could not read the timeline at all.
export const timelineUnavailable = writable(false);
/// Recording paused. Live state comes with the pause command; the toggle is
/// optimistic under vite.
export const paused = writable(false);

/// True when the last pause or resume did not reach the daemon, so the switch
/// went back to what it was. Recording that continues while the surface says
/// "paused" is the worst failure this app has, and it is the one the optimistic
/// flip produced on every real session: the daemon has no pause command at all,
/// so the call always threw and the toggle always stayed where the user put it.
export const pauseUnavailable = writable(false);

function localMidnight(unix: number): number {
  const d = new Date(unix * 1000);
  d.setHours(0, 0, 0, 0);
  return Math.floor(d.getTime() / 1000);
}

/// The spine's events as one flat list (sessions unrolled), newest first -
/// the projects detail reuses these for its recent-activity block.
export function flatEvents(list: TimelineDay[]): TimelineEvent[] {
  const out: TimelineEvent[] = [];
  for (const day of list) {
    for (const item of day.items) {
      if (item.kind === "event") out.push(item.event);
      else out.push(...item.session.events);
    }
  }
  return out.sort((a, b) => b.at - a.at);
}

/// Group a flat item list into days, newest first.
export function groupByDay(items: TimelineItem[]): TimelineDay[] {
  const at = (it: TimelineItem) => (it.kind === "event" ? it.event.at : it.session.to);
  const sorted = [...items].sort((a, b) => at(b) - at(a));
  const out: TimelineDay[] = [];
  for (const item of sorted) {
    const date = localMidnight(at(item));
    const day = out[out.length - 1];
    if (day && day.date === date) day.items.push(item);
    else out.push({ date, items: [item] });
  }
  return out;
}

/// "Today" / "Yesterday" in the caller's locale, capitalised for a header.
///
/// One function because there were two: `dayLabel` was moved onto
/// `Intl.RelativeTimeFormat` and left a comment explaining why the
/// `locale.startsWith("de")` ladder was wrong - "a ladder that knew exactly two
/// languages and handed English to everyone else" - while `dayLabelShort`, three
/// lines below that comment, still had the ladder. Sharing the word is what
/// stops the two drifting again.
function relativeDay(days: 0 | -1, locale: string): string {
  const word = new Intl.RelativeTimeFormat(locale, { numeric: "auto" }).format(days, "day");
  // The locale data returns it mid-sentence; a label is capitalised.
  return word.charAt(0).toLocaleUpperCase(locale) + word.slice(1);
}

/// "Today", "Yesterday", or the written date, for the day headers and the
/// scrub grip.
///
/// The two words come from `Intl.RelativeTimeFormat` with `numeric: "auto"`,
/// which is what it is for. This used to be `locale.startsWith("de") ? "Heute" :
/// "Today"` - a ladder that knew exactly two languages and handed English to
/// everyone else, which is a worse failure than a missing translation because it
/// looks deliberate. Anything older than yesterday keeps the written date: a
/// timeline wants "Thursday, 24 June", not "3 days ago".
export function dayLabel(date: number, locale: string): string {
  const today = localMidnight(Math.floor(Date.now() / 1000));
  const days = Math.round((date - today) / 86400);
  if (days === 0 || days === -1) {
    return relativeDay(days, locale);
  }
  return new Date(date * 1000).toLocaleDateString(locale, { weekday: "long", day: "numeric", month: "long" });
}

/// Clock time for an event row, tabular.
export function clock(unix: number, locale: string): string {
  return new Date(unix * 1000).toLocaleTimeString(locale, { hour: "numeric", minute: "2-digit" });
}

/// The short form for the scrub grip ("Thu 24"), so the grip stays compact at
/// the rail's edges; the full label lives in aria-valuetext and the day heads.
export function dayLabelShort(date: number, locale: string): string {
  const today = localMidnight(Math.floor(Date.now() / 1000));
  if (date === today) return relativeDay(0, locale);
  if (date === today - 86400) return relativeDay(-1, locale);
  return new Date(date * 1000).toLocaleDateString(locale, { weekday: "short", day: "numeric" });
}

const now = Math.floor(Date.now() / 1000);
const dayAgo = (d: number, h: number, m = 0): number => {
  const base = new Date(now * 1000);
  base.setHours(h, m, 0, 0);
  return Math.floor(base.getTime() / 1000) - d * 86400;
};

let seq = 0;
function ev(kind: TimelineKind, verb: string, object: string, source: string, at: number, project?: string): TimelineEvent {
  return { id: `tl-${++seq}`, kind, verb, object, source, at, project };
}

// The fixture spine: several days, mixed kinds, two reconstructed sessions,
// bridged imports - dense enough that the recall value reads (the graph's
// worth is context, not an activity gimmick).
//
// i18n-foreign: the project and document names are the user's own words. The
// graph hands them over in whatever language they were written in, so a fixture
// standing in for them is not ours to translate either - the same reason the
// library, search and capsule fixtures carry this marker. The verbs beside them
// ARE ours and go through the catalog (`k.tl.verb.*`), which is the line.
function fixture(): TimelineItem[] {
  const items: TimelineItem[] = [
    { kind: "event", event: ev("opened", "k.tl.verb.opened", "Quarterly report.pdf", "Files", dayAgo(0, 9, 12)) },
    { kind: "event", event: ev("ran", "k.tl.verb.ran", "cargo build", "Terminal", dayAgo(0, 9, 41), "Arlen OS") },
    { kind: "event", event: ev("agent", "k.tl.verb.tagged", "3 files to Thesis", "Assistant", dayAgo(0, 10, 5), "Thesis") },
    {
      kind: "session",
      session: {
        id: "s-1",
        title: "Website redesign",
        from: dayAgo(0, 14, 10),
        to: dayAgo(0, 16, 40),
        events: [
          ev("opened", "k.tl.verb.opened", "landing.fig", "Files", dayAgo(0, 14, 12), "Website redesign"),
          ev("edited", "k.tl.verb.edited", "hero.css", "Text editor", dayAgo(0, 14, 55), "Website redesign"),
          ev("focus", "k.tl.verb.focused", "Browser, localhost:5173", "Shell", dayAgo(0, 15, 30), "Website redesign"),
          ev("edited", "k.tl.verb.edited", "hero.css", "Text editor", dayAgo(0, 16, 22), "Website redesign"),
        ],
      },
    },
    { kind: "event", event: ev("edited", "k.tl.verb.edited", "chapter-3.md", "Text editor", dayAgo(1, 11, 20), "Thesis") },
    { kind: "event", event: ev("imported", "k.tl.verb.imported", "12 papers from Zotero", "Zotero bridge", dayAgo(1, 13, 2)) },
    { kind: "event", event: ev("opened", "k.tl.verb.opened", "Attention Is All You Need.pdf", "Files", dayAgo(1, 13, 15), "Thesis") },
    {
      kind: "session",
      session: {
        id: "s-2",
        title: "Arlen OS",
        from: dayAgo(2, 9, 30),
        to: dayAgo(2, 12, 15),
        events: [
          ev("focus", "k.tl.verb.focused", "Terminal, ~/Repositories/arlen", "Shell", dayAgo(2, 9, 32), "Arlen OS"),
          ev("edited", "k.tl.verb.edited", "compositor.toml", "Text editor", dayAgo(2, 10, 8), "Arlen OS"),
          ev("ran", "k.tl.verb.ran", "just dev", "Terminal", dayAgo(2, 10, 12), "Arlen OS"),
          ev("opened", "k.tl.verb.opened", "design-system.md", "Files", dayAgo(2, 11, 47), "Arlen OS"),
        ],
      },
    },
    { kind: "event", event: ev("imported", "k.tl.verb.imported", "Re: review notes", "Thunderbird bridge", dayAgo(3, 8, 44)) },
    { kind: "event", event: ev("opened", "k.tl.verb.opened", "The Rust Programming Language", "Library", dayAgo(3, 21, 6)) },
    { kind: "event", event: ev("ran", "k.tl.verb.ran", "git push", "Terminal", dayAgo(4, 17, 58), "Arlen OS") },
    { kind: "event", event: ev("edited", "k.tl.verb.edited", "notes.md", "Text editor", dayAgo(4, 18, 30)) },
  ];
  return items;
}

/// Load the spine. Live: `knowledge_timeline` (the FUSE timeline + typed
/// reads, a coder seam); fixture under vite.
export async function loadTimeline(): Promise<void> {
  try {
    const items = await invoke<TimelineItem[]>("knowledge_timeline", {});
    days.set(groupByDay(items));
    timelineMocked.set(false);
    timelineUnavailable.set(false);
  } catch {
    if (import.meta.env.DEV) {
      days.set(groupByDay(fixture()));
      timelineMocked.set(true);
      timelineUnavailable.set(false);
      return;
    }
    // This is the app's landing view, and the fixture is a week of dated
    // activity attributed to this machine - "ran Terminal", "imported Zotero
    // bridge", "edited Text editor", each with a time. A label above a week of
    // invented history is not enough when the history is the whole page.
    days.set([]);
    timelineMocked.set(false);
    timelineUnavailable.set(true);
  }
}

/// Pause or resume recording. Live: `knowledge_timeline_pause` (seam); the
/// optimistic flip stands under vite, behind the mocked banner.
export async function setPaused(value: boolean): Promise<void> {
  const previous = get(paused);
  paused.set(value);
  pauseUnavailable.set(false);
  try {
    await invoke("knowledge_timeline_pause", { paused: value });
  } catch {
    if (import.meta.env.DEV) return; // no backend under vite
    // The daemon has no pause today, so this is the live path: put the switch
    // back and say so. Leaving it flipped would tell someone their activity is
    // no longer being recorded while it is.
    paused.set(previous);
    pauseUnavailable.set(true);
  }
}

/// Export the recorded timeline to a file, returning where it went.
///
/// The path matters as much as the success: an export that says "done" without
/// saying where leaves the user hunting for their own history.
export async function exportTimeline(): Promise<string | null> {
  try {
    return await invoke<string>("knowledge_timeline_export", {});
  } catch {
    return null;
  }
}

/// Delete a recorded range for good. Live: `knowledge_timeline_delete` (seam);
/// under vite the fixture drops the range locally, behind the mocked banner.
///
/// Returns whether the delete actually happened, and on a real failure it leaves
/// the timeline ALONE. The catch used to drop the range locally whatever the
/// command did, so a delete that never reached the graph cleared the screen and
/// the user was told their history was gone while it sat on disk. That is the
/// worst place in the app for a comfortable lie: the control says "for good", and
/// the person clicking it is doing so because they mean it. The DEV branch keeps
/// the local drop because under vite the whole store is a fixture behind the
/// mocked banner - there is no graph to have failed.
export async function deleteRange(fromUnix: number): Promise<boolean> {
  try {
    await invoke("knowledge_timeline_delete", { from: fromUnix });
    await loadTimeline();
    return true;
  } catch {
    if (!import.meta.env.DEV) {
      return false;
    }
    days.update((d) =>
      d
        ? d
            .map((day) => ({
              ...day,
              items: day.items.filter((it) => (it.kind === "event" ? it.event.at : it.session.to) < fromUnix),
            }))
            .filter((day) => day.items.length > 0)
        : d
    );
    return true;
  }
}
