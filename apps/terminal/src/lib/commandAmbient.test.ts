import { describe, expect, it } from "vitest";
import { commandAmbient, runningEffect, LONG_RUNNING_MS } from "./commandAmbient";
import type { AmbientParams } from "@arlen/tauri-plugin-shell";

/// A hand-driven timer, so the rule is tested rather than five real seconds.
function harness() {
  const published: (AmbientParams | null)[] = [];
  let pending: (() => void) | null = null;
  let lastMs = 0;
  let cleared = 0;
  const driver = commandAmbient(
    (p) => published.push(p),
    (fn, ms) => {
      pending = fn;
      lastMs = ms;
      return 1;
    },
    () => {
      pending = null;
      cleared += 1;
    },
  );
  return {
    driver,
    published,
    fire: () => pending?.(),
    get armed() {
      return pending !== null;
    },
    get waitedMs() {
      return lastMs;
    },
    get cleared() {
      return cleared;
    },
  };
}

describe("commandAmbient", () => {
  /// The noise case, and the reason the threshold exists: a tint for every `ls`
  /// is the whole screen saying something about nothing.
  it("says nothing about a command that finishes quickly", () => {
    const h = harness();
    h.driver.started();
    h.driver.ended();
    expect(h.published).toEqual([]);
    expect(h.armed).toBe(false);
  });

  it("pulses once the command has been going long enough", () => {
    const h = harness();
    h.driver.started();
    expect(h.waitedMs).toBe(LONG_RUNNING_MS);
    h.fire();
    expect(h.published).toEqual([runningEffect()]);
  });

  it("takes it down when the command ends", () => {
    const h = harness();
    h.driver.started();
    h.fire();
    h.driver.ended();
    expect(h.published).toEqual([runningEffect(), null]);
  });

  /// A second exec-start without an end restarts the wait rather than stacking
  /// timers - otherwise a nested shell leaves a timer that fires after the
  /// command it belonged to has gone.
  it("restarts the wait rather than stacking timers", () => {
    const h = harness();
    h.driver.started();
    h.driver.started();
    expect(h.cleared).toBe(1);
    expect(h.armed).toBe(true);
  });

  it("gives up its timer when the window goes", () => {
    const h = harness();
    h.driver.started();
    h.driver.dispose();
    expect(h.armed).toBe(false);
    expect(h.published).toEqual([]);
  });
});

describe("runningEffect", () => {
  /// The cap is 0.5 and this is well under it on purpose: a wash you have to
  /// look for is the right strength for "still going".
  it("stays well under the intensity cap", () => {
    expect(runningEffect().intensity).toBeLessThan(0.5);
  });

  /// The command line never leaves this window - the reason names the activity,
  /// not what was typed.
  it("names the activity and not the command", () => {
    expect(runningEffect().reason).toBe("a command is running");
  });

  /// No auto-clear: a command that runs for an hour pulses for an hour, and the
  /// clear comes from it ending. An auto-clear would say the work had finished.
  it("does not expire on its own", () => {
    expect(runningEffect().autoClearMs).toBeUndefined();
  });
});
