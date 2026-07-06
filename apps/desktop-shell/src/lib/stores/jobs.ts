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
  items?: JobItem[];
}

const MOCK_JOBS: Job[] = [
  {
    id: "fm-copy",
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

/// Load the current jobs. Live: `list_jobs` + the event feed; fixture under vite.
export async function pollJobs(): Promise<void> {
  try {
    jobs.set(await invoke<Job[]>("list_jobs"));
  } catch {
    jobs.set(MOCK_JOBS);
  }
}

/// Cancel a job (a clean cancel, per the Killable flag). Live: `cancel_job`.
export async function cancelJob(id: string): Promise<void> {
  jobs.update((list) => list.filter((j) => j.id !== id));
  try {
    await invoke("cancel_job", { id });
  } catch {
    // No daemon under vite: the optimistic removal stands.
  }
}

/// Pause a suspendable job. Live: `pause_job`.
export async function pauseJob(id: string): Promise<void> {
  jobs.update((list) => list.map((j) => (j.id === id ? { ...j, state: "paused" } : j)));
  try {
    await invoke("pause_job", { id });
  } catch {
    // mock
  }
}

/// Resume a paused job. Live: `resume_job`.
export async function resumeJob(id: string): Promise<void> {
  jobs.update((list) => list.map((j) => (j.id === id ? { ...j, state: "running" } : j)));
  try {
    await invoke("resume_job", { id });
  } catch {
    // mock
  }
}
