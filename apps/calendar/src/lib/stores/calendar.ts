/// The calendar's data spine. The READ path is the app's real wire, unchanged:
/// `calendar_agenda` (service first, files second, a launched file alone),
/// re-read on `arlen://calendar-changed`. Under plain vite a fixture agenda
/// stands in - marked as an example on the surface - so the views can be
/// designed and driven without a host.
///
/// The WRITE path is the intended `calendar_create_event` seam: the coder
/// writes one VEVENT file into the store directory, at which point the watcher
/// and the reminder daemon pick it up with no further wiring. Until it exists
/// a live press answers with an honest refusal; the fixture applies locally so
/// the whole flow drives.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { tauriAvailable } from "$lib/tauri";

/// One expanded occurrence, exactly as the backend serialises it (snake_case).
export type AgendaEvent = {
  uid: string;
  summary: string;
  location: string;
  date: string;
  time: string | null;
  end_time: string | null;
  kind: string;
  tzid: string | null;
  repeats: boolean;
  every: string | null;
  every_n: number;
  on_days: string[];
  expanded: boolean;
};

/// The agenda answer, verbatim.
export type Agenda = {
  events: AgendaEvent[];
  directory: string;
  directory_exists: boolean;
  files: number;
  unreadable: number;
  service_running: boolean;
};

/// A new event as the form writes it; the seam's documented draft shape.
export interface EventDraft {
  summary: string;
  /// YYYY-MM-DD in the event's own local terms.
  date: string;
  allDay: boolean;
  /// HH:MM, present when not all-day.
  time: string | null;
  endTime: string | null;
  location: string;
  repeat: "none" | "daily" | "weekly";
  /// mon..sun, weekly only.
  onDays: string[];
}

// ---------------------------------------------------------------------------
// Date helpers. Dates are ALWAYS built from parts: `new Date("2026-08-21")`
// is UTC midnight and shows the day before west of Greenwich (pinned by the
// coder on this app, 22 Aug).
// ---------------------------------------------------------------------------

/// A local date as YYYY-MM-DD.
export function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/// The parts of a YYYY-MM-DD, as a local Date.
export function parseYmd(s: string): Date {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
}

/// The date `n` days after `date` (n may be negative).
export function addDays(date: string, n: number): string {
  const d = parseYmd(date);
  d.setDate(d.getDate() + n);
  return ymd(d);
}

/// The Monday of the week `date` falls in.
export function startOfWeek(date: string): string {
  const d = parseYmd(date);
  const shift = (d.getDay() + 6) % 7;
  return addDays(date, -shift);
}

/// Minutes since midnight for an HH:MM string.
export function minutesOf(time: string): number {
  const [h, m] = time.split(":").map(Number);
  return h * 60 + (m || 0);
}

/// One positioned block in a day column: overlapping events split the column.
export interface DayBlock {
  event: AgendaEvent;
  startMin: number;
  endMin: number;
  col: number;
  cols: number;
}

/// Lay out one day's TIMED events: sort by start, assign greedy columns inside
/// each overlapping cluster, and give every member of a cluster the cluster's
/// width so side-by-side events actually sit side by side.
export function layoutDay(events: AgendaEvent[]): DayBlock[] {
  const timed = events
    .filter((e) => e.time !== null)
    .map((e) => {
      const start = minutesOf(e.time as string);
      const end = e.end_time ? Math.max(minutesOf(e.end_time), start + 15) : start + 30;
      return { event: e, startMin: start, endMin: end, col: 0, cols: 1 };
    })
    .sort((a, b) => a.startMin - b.startMin || a.endMin - b.endMin);

  let cluster: DayBlock[] = [];
  let clusterEnd = -1;
  const close = () => {
    const cols = Math.max(1, ...cluster.map((b) => b.col + 1));
    for (const b of cluster) b.cols = cols;
    cluster = [];
  };
  for (const b of timed) {
    if (cluster.length > 0 && b.startMin >= clusterEnd) close();
    const taken = new Set(cluster.filter((o) => o.endMin > b.startMin).map((o) => o.col));
    let col = 0;
    while (taken.has(col)) col += 1;
    b.col = col;
    cluster.push(b);
    clusterEnd = Math.max(clusterEnd, b.endMin);
  }
  if (cluster.length > 0) close();
  return timed;
}

// ---------------------------------------------------------------------------
// Fixture: a plausible week around today, covering what the grid must carry -
// an overlap, an all-day, a zoned time, expanded and refused repeats, a long
// block. English on purpose; the banner says it is an example.
// ---------------------------------------------------------------------------

function fixtureAgenda(): Agenda {
  const today = ymd(new Date());
  const monday = startOfWeek(today);
  const ev = (partial: Partial<AgendaEvent> & { uid: string; summary: string; date: string }): AgendaEvent => ({
    location: "",
    time: null,
    end_time: null,
    kind: "floating",
    tzid: null,
    repeats: false,
    every: null,
    every_n: 1,
    on_days: [],
    expanded: false,
    ...partial,
  });
  const events: AgendaEvent[] = [];
  for (let i = 0; i < 5; i++) {
    events.push(
      ev({
        uid: "standup",
        summary: "Standup",
        date: addDays(monday, i),
        time: "09:00",
        end_time: "09:15",
        repeats: true,
        every: "weekly",
        on_days: ["mon", "tue", "wed", "thu", "fri"],
        expanded: true,
      }),
    );
  }
  events.push(
    ev({ uid: "review", summary: "Design review", date: today, time: "10:00", end_time: "11:30", location: "Studio" }),
    ev({ uid: "oneone", summary: "1:1 Jonas", date: today, time: "10:30", end_time: "11:00" }),
    ev({ uid: "nyc", summary: "Call with New York", date: today, time: "16:00", end_time: "16:45", kind: "zoned", tzid: "America/New_York" }),
    ev({ uid: "holiday", summary: "Public holiday", date: addDays(monday, 3), kind: "day" }),
    ev({ uid: "workshop", summary: "Print workshop", date: addDays(monday, 4), time: "14:00", end_time: "17:30", location: "Werkstatt" }),
    ev({ uid: "birthday", summary: "Mara's birthday", date: addDays(monday, 5), kind: "day", repeats: true, every: "yearly", expanded: true }),
    ev({ uid: "planning", summary: "Planning breakfast", date: addDays(monday, 7 + 1), time: "09:30", end_time: "10:30", location: "Cafe am Eck" }),
    ev({ uid: "rent", summary: "Rent due", date: addDays(monday, 9), repeats: true, every: null, expanded: false }),
    ev({ uid: "dentist", summary: "Dentist", date: addDays(monday, 16), time: "08:15", end_time: "09:00" }),
    ev({ uid: "concert", summary: "Concert", date: addDays(monday, -3), time: "20:00", end_time: "22:30", location: "Stadthalle" }),
  );
  events.sort((a, b) => a.date.localeCompare(b.date) || (a.time ?? "").localeCompare(b.time ?? ""));
  return {
    events,
    directory: "~/.local/share/arlen/calendars",
    directory_exists: true,
    files: 3,
    unreadable: 0,
    service_running: true,
  };
}

/// The agenda (fixture or live).
export const agenda = writable<Agenda | null>(null);
/// True while the agenda is the FIXTURE - the surface says so.
export const calendarMocked = writable(false);

/// The named read failure, decoded by the page (its wiring, unchanged).
export type AgendaFailure =
  | { problem: "no-home" }
  | { problem: "unreadable"; why: string }
  | { problem: "other"; reason: string };

/// Read the agenda: the service first, the files second, a launched file
/// alone. Throws the backend's named problem for the page to decode; under
/// plain vite the fixture stands in.
export async function loadAgenda(file: string | null): Promise<void> {
  if (!tauriAvailable) {
    agenda.set(fixtureAgenda());
    calendarMocked.set(true);
    return;
  }
  agenda.set(await invoke<Agenda>("calendar_agenda", { file }));
  calendarMocked.set(false);
}

/// Create one event. Live: the intended `calendar_create_event(draft)` writing
/// a VEVENT into the store directory (the watcher re-reads, the daemon arms
/// the reminder). Fixture: applied locally so the flow drives. Returns the
/// refusal text when the press could not do the true thing.
export async function createEvent(draft: EventDraft): Promise<string | null> {
  try {
    await invoke("calendar_create_event", { draft });
    return null;
  } catch (e) {
    let mocked = false;
    calendarMocked.update((m) => ((mocked = m), m));
    if (!mocked) return String(e);
    agenda.update((a) => {
      if (!a) return a;
      const base: AgendaEvent = {
        uid: `draft-${Date.now()}`,
        summary: draft.summary || "(untitled)",
        location: draft.location,
        date: draft.date,
        time: draft.allDay ? null : draft.time,
        end_time: draft.allDay ? null : draft.endTime,
        kind: draft.allDay ? "day" : "floating",
        tzid: null,
        repeats: draft.repeat !== "none",
        every: draft.repeat === "none" ? null : draft.repeat,
        every_n: 1,
        on_days: draft.repeat === "weekly" ? draft.onDays : [],
        expanded: draft.repeat !== "none",
      };
      const events = [...a.events];
      if (draft.repeat === "none") events.push(base);
      else {
        // A small local expansion (four weeks ahead) so the fixture grid shows
        // the rule; the real expansion is the core's job.
        for (let i = 0; i < 28; i++) {
          const d = addDays(draft.date, i);
          const dow = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"][parseYmd(d).getDay()];
          const daily = draft.repeat === "daily";
          const weekly = draft.repeat === "weekly" && (draft.onDays.length === 0 ? d === draft.date || parseYmd(d).getDay() === parseYmd(draft.date).getDay() : draft.onDays.includes(dow));
          if (daily || weekly) events.push({ ...base, date: d });
        }
      }
      events.sort((x, y) => x.date.localeCompare(y.date) || (x.time ?? "").localeCompare(y.time ?? ""));
      return { ...a, events };
    });
    return null;
  }
}
