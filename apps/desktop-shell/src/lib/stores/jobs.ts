/// The Activity/Jobs feed (job-progress-surface.md): the shell-owned aggregator of
/// long-running background work - file operations, package installs, model
/// downloads, transfers - shown as a dedicated zone at the top of the notifications
/// popover. A job is live, progressing, and cancelable; a notification is a past
/// event. Closes the acute gap that the file manager reports no progress today.
///
/// Mock-vs-live: fixture-backed. The JobView feed (the notification-daemon extended
/// into a KDE-JobViewV3-mirror job server + the producers reporting progress) is a
/// coder seam; the `list_jobs` query + cancel/pause/resume commands + the event feed
/// are not built. Under vite the store serves a fixture set so the zone renders. The
/// shell owns the threshold/min-dwell visibility (a job shows once it passes ~1.5s).

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { tauriAvailable } from "$lib/tauri";

/// A job's lifecycle state (mirrors the JobView state enum).
export type JobState =
  | "running"
  | "paused"
  | "impeded"
  | "error_recoverable"
  | "error_fatal"
  | "done";

/// One real-unit metric ("84 of 240 files"). The consumer derives the sentence;
/// the feed never pre-bakes a percentage.
export interface JobMetric {
  processed: number;
  total: number;
  unit: string;
}

/// One item inside a composite job, for the expandable per-item list (never hide
/// the per-file names behind a lone aggregate bar).
export interface JobItem {
  name: string;
  done: boolean;
}


/// A long-running operation, as the zone renders it.
export interface Job {
  id: string;
  title: string;
  /// The attested producer app id + a friendly label.
  appId: string;
  appLabel: string;
  /// A monotonic 0..1 fraction (never backwards), kept separate from the ETA.
  fraction: number;
  state: JobState;
  metrics: JobMetric[];
  /// A coarse ETA ("about 3 minutes"), never false hh:mm:ss precision.
  etaText?: string;
  killable: boolean;
  suspendable: boolean;
  /// A message for the non-running error/impeded states.
  error?: string;
  /// The host a network job reaches (no-silent-egress transparency).
  egressHost?: string;
  /// The entries this job works through, with the finished ones marked.
  ///
  /// The zone expands its aggregate bar into these: showing only a total that
  /// hides the file names is the mistake the plan names outright. Which are done
  /// is DERIVED in the shell backend from the names and the count, not carried
  /// per item - two copies of one fact eventually disagree.
  items: JobItem[];
  /// When the producer started it, epoch micros.
  ///
  /// The zone owns the visibility threshold - a job shows once it has run long
  /// enough to be worth a row - and this is what that decision is made from. The
  /// daemon deliberately does not decide it: a threshold in the producer would
  /// be every producer inventing its own idea of "worth showing".
  startedAt: number;
}

// i18n-foreign: a job's title is written by whichever daemon is doing the work
// and names the user's own files - "Copying 240 photos to USB" is a sentence
// about their photos, not a label of ours. The fixture stands in for that, so it
// is not ours to translate either; making the daemons send an id instead is the
// same design question the consent summary raises.
const MOCK_JOBS: Job[] = [
  {
    id: "fm-copy",
    startedAt: 1000000,
    title: "Copying 240 photos to USB",
    appId: "org.arlen.files",
    appLabel: "Files",
    fraction: 0.35,
    state: "running",
    metrics: [
      { processed: 84, total: 240, unit: "files" },
      { processed: 120, total: 340, unit: "MB" },
    ],
    etaText: "about 2 minutes",
    killable: true,
    suspendable: true,
    items: [
      { name: "IMG_2103.jpg", done: true },
      { name: "IMG_2104.jpg", done: true },
      { name: "IMG_2105.jpg", done: false },
      { name: "IMG_2106.jpg", done: false },
    ],
  },
  {
    id: "model-pull",
    items: [],
    startedAt: 2000000,
    title: "Downloading the language model",
    appId: "org.arlen.assistant",
    appLabel: "Assistant",
    fraction: 0.22,
    state: "running",
    metrics: [{ processed: 1300, total: 5900, unit: "MB" }],
    etaText: "about 6 minutes",
    killable: true,
    suspendable: true,
    egressHost: "huggingface.co",
  },
  {
    id: "transfer",
    items: [],
    startedAt: 3000000,
    title: "Sending files to your laptop",
    appId: "org.arlen.files",
    appLabel: "Files",
    fraction: 0.5,
    state: "paused",
    metrics: [{ processed: 5, total: 12, unit: "files" }],
    killable: true,
    suspendable: true,
  },
  {
    id: "convert",
    items: [],
    startedAt: 4000000,
    title: "Converting clip.mp4",
    appId: "org.arlen.media",
    appLabel: "Media",
    fraction: 0.8,
    state: "error_recoverable",
    metrics: [{ processed: 48, total: 60, unit: "seconds" }],
    error: "Ran out of disk space. Free some room and retry.",
    killable: true,
    suspendable: false,
  },
  {
    id: "fm-done",
    items: [],
    startedAt: 5000000,
    title: "Copied 18 files to Documents",
    appId: "org.arlen.files",
    appLabel: "Files",
    fraction: 1,
    state: "done",
    metrics: [{ processed: 18, total: 18, unit: "files" }],
    killable: false,
    suspendable: false,
  },
];

/// The jobs on screen now (fixture until the JobView feed lands).
export const jobs = writable<Job[]>([]);

/// True while the feed is the MOCK, not real background work. The zone shows a
/// titled job with real-unit metrics ("84 of 240 files"), an ETA, per-file names
/// and a Cancel button - unlabelled it reads as a copy actually in flight, and a
/// user could cancel it, or unplug the drive believing it is 35% done.
export const mocked = writable(false);

/// The last action failure, for the zone to show. Empty when all is well.
/// The keys this store may hold. A UNION rather than `string`, because a
/// `string` is also what `String(e)` and an English sentence written here both
/// are - and both were written into stores exactly like this one tonight. Naming
/// them makes a wrong write a compile error rather than something a check has to
/// find afterwards.
export type JobMessage =
  | ""
  | "sh.jobs.unavailable"
  | "sh.job.notCancelled"
  | "sh.job.notPaused"
  | "sh.job.notResumed";

/// The message KEY of the last refusal, or "" for none.
///
/// A key rather than a resolved sentence, so the line follows a locale change
/// while it is on screen: `get(t)(...)` freezes the wording at the moment of
/// failure, and this line stays up until the next action replaces it.
export const lastError = writable<JobMessage>("");

/// Load the current jobs. Live: `list_jobs` + the event feed; fixture under vite.
export async function pollJobs(): Promise<void> {
  try {
    jobs.set(await invoke<Job[]>("list_jobs"));
    mocked.set(false);
  } catch {
    if (!tauriAvailable) {
      jobs.set(MOCK_JOBS);
      mocked.set(true);
      return;
    }
    // A real session: show no jobs rather than invented ones. These rows carry
    // actions - cancel, retry - so a fabricated job is a button that acts on
    // something that does not exist, and the zone's own `lastError` is where the
    // failure belongs.
    jobs.set([]);
    mocked.set(false);
    lastError.set("sh.jobs.unavailable");
  }
}

/// Follow the daemon's live feed.
///
/// `pollJobs` is the snapshot a zone opens with; this is what keeps it moving.
/// The two carry the SAME row - the shell builds it once from the wire and sends
/// it both ways - so a job cannot read one way in the list and another as it
/// updates.
///
/// A job arrives with `removed` when it finished or was cancelled, and it leaves
/// the feed at that point: the transient "done" receipt is the zone's to show,
/// and a store that kept finished rows would make the list a history rather than
/// a picture of what is running.
export async function watchJobs(): Promise<UnlistenFn | null> {
  if (!tauriAvailable) return null;
  return listen<{ job: Job; removed: boolean }>("notification:job", (event) => {
    const { job, removed } = event.payload;
    jobs.update((list) => {
      const rest = list.filter((j) => j.id !== job.id);
      // Oldest first, the same order `list_jobs` returns, so the snapshot and
      // the feed cannot disagree about where a row sits. A row that moves as its
      // progress changes is a row nobody can click.
      return removed
        ? rest
        : [...rest, job].sort((a, b) => a.startedAt - b.startedAt || a.id.localeCompare(b.id));
    });
  });
}

/// Drive one job action optimistically, then reconcile with the daemon.
///
/// A REAL refusal restores the previous feed and says so. Swallowing it would
/// report a cancelled copy that is still running - the same false confirmation of
/// a destructive action the task manager had. Without the runtime there is no
/// daemon to refuse, so the optimistic mock stands.
/// `failure` is a message KEY, not a sentence.
///
/// It used to be an English string written here - "Could not cancel that job" -
/// with `String(e)` appended, and the zone rendered the pair verbatim. So the
/// only line a person sees when a cancel is refused was written in a TypeScript
/// store where no catalogue can reach it, followed by whatever the daemon
/// formatted. The born-translatable lint reads a prose literal assigned to an
/// `*error`/`*message` NAME; passed as an argument called `failure`, this slipped
/// past it.
async function driveJob(
  id: string,
  apply: (list: Job[]) => Job[],
  cmd: string,
  failure: JobMessage,
): Promise<void> {
  let previous: Job[] = [];
  jobs.update((list) => {
    previous = list;
    return apply(list);
  });
  try {
    await invoke(cmd, { id });
  } catch (e) {
    if (tauriAvailable) {
      jobs.set(previous);
      // The daemon's own words name a command and carry an errno. They go where
      // whoever debugs this will read them; the zone gets the sentence.
      console.warn(`shell: ${cmd} refused`, e);
      lastError.set(failure);
    }
  }
}

/// Cancel a job (a clean cancel, per the Killable flag). Live: `cancel_job`.
export async function cancelJob(id: string): Promise<void> {
  await driveJob(id, (l) => l.filter((j) => j.id !== id), "cancel_job", "sh.job.notCancelled");
}

/// Pause a suspendable job. Live: `pause_job`.
export async function pauseJob(id: string): Promise<void> {
  await driveJob(
    id,
    (l) => l.map((j) => (j.id === id ? { ...j, state: "paused" } : j)),
    "pause_job",
    "sh.job.notPaused",
  );
}

/// Resume a paused job. Live: `resume_job`.
export async function resumeJob(id: string): Promise<void> {
  await driveJob(
    id,
    (l) => l.map((j) => (j.id === id ? { ...j, state: "running" } : j)),
    "resume_job",
    "sh.job.notResumed",
  );
}
