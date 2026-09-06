/// A finished job must stay readable for a moment.
///
/// The real defect this guards: every producer says how a job ended and removes
/// it in the same breath - `set_state("done")` or `set_state("error-fatal", why)`
/// and then `finish`, microseconds apart - so a store that drops the row on the
/// removal shows the receipt for no frames at all. The message on a FAILED
/// install was the one nobody ever saw.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";

// A Tauri runtime has to look present before the store module is read: the
// availability flag is a module-level const, so mocking it afterwards is too late.
vi.mock("$lib/tauri", () => ({ tauriAvailable: true }));

let handler: ((e: { payload: { job: unknown; removed: boolean } }) => void) | null = null;
const unlisten = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (e: { payload: { job: unknown; removed: boolean } }) => void) => {
    handler = cb;
    return Promise.resolve(unlisten);
  },
}));

const { jobs, watchJobs } = await import("./jobs");

const row = (id: string, state: string) => ({
  id,
  title: "Installing notes-1.2",
  appId: "arlen-installd",
  appLabel: "arlen-installd",
  fraction: 0,
  determinate: false,
  state,
  metrics: [],
  killable: false,
  suspendable: false,
  items: [],
  startedAt: 1,
  error: state === "error_fatal" ? "package not found" : undefined,
});

describe("a finished job's receipt", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    jobs.set([]);
    handler = null;
  });

  it("stays on the list long enough to be read, then goes", async () => {
    const stop = await watchJobs();
    handler!({ payload: { job: row("j1", "running"), removed: false } });
    expect(get(jobs)).toHaveLength(1);

    // The producer's last two calls: the verdict, then the removal.
    handler!({ payload: { job: row("j1", "error_fatal"), removed: false } });
    handler!({ payload: { job: row("j1", "error_fatal"), removed: true } });

    // Still there, carrying WHY - this is the whole point.
    const held = get(jobs);
    expect(held).toHaveLength(1);
    expect(held[0].state).toBe("error_fatal");
    expect(held[0].error).toBe("package not found");

    vi.advanceTimersByTime(7999);
    expect(get(jobs)).toHaveLength(1);
    vi.advanceTimersByTime(2);
    expect(get(jobs)).toHaveLength(0);
    stop?.();
  });

  it("does not leave a row behind when a producer finishes twice", async () => {
    const stop = await watchJobs();
    handler!({ payload: { job: row("j1", "done"), removed: true } });
    handler!({ payload: { job: row("j1", "done"), removed: true } });
    vi.advanceTimersByTime(9000);
    expect(get(jobs)).toHaveLength(0);
    stop?.();
  });

  it("drops its pending timers when the shell stops watching", async () => {
    const stop = await watchJobs();
    handler!({ payload: { job: row("j1", "done"), removed: true } });
    stop?.();
    expect(unlisten).toHaveBeenCalled();
    // Nothing rendering from the store any more, so nothing may edit it either.
    const before = get(jobs).length;
    vi.advanceTimersByTime(9000);
    expect(get(jobs)).toHaveLength(before);
  });
});
