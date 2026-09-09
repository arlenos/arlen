/// A slow pulse on the desktop while a command is still going.
///
/// `shell.ambient` is the peripheral-awareness surface: a wash of colour that
/// says something is happening without asking to be read. Nothing in the tree
/// published one, and `ambient-api.md`'s own two examples - a build running,
/// tests failing - are both this shape. The terminal is where they actually
/// happen here.
///
/// **It renders whether or not this window is focused**, since 9 September:
/// FA1's focused-only clause was dropped because it made the tint appear exactly
/// when the app's own window already said everything and stay silent when it did
/// not. So a build left running in a terminal on another workspace is now the
/// case this serves, and the one it was written for - something running in a
/// window you are watching but not reading - still works.
///
/// **THE THRESHOLD IS THE ONE INVENTED NUMBER HERE, and it is load-bearing.**
/// Pulsing on every command would tint the screen for `ls`, which is noise
/// wearing the clothes of information, and noise on this surface is worse than
/// silence because it cannot be ignored - it is the whole screen. So the pulse
/// starts only once a command has been going long enough that somebody might
/// have stopped waiting for it. Five seconds is a judgement, not a measurement:
/// it is past the point where a person watches a command finish and short of
/// the point where they have forgotten it. If it ever wants to be a setting, it
/// belongs beside the terminal's own preferences rather than in the ambient API,
/// which has no opinion about what "long" means for any particular app.
///
/// **NO COMMAND, NO ARGUMENTS, NOTHING.** The same rule `graphInput.ts` states
/// at length: this app sees the whole command line and publishing it would be
/// the one place where saying more makes the system worse. An ambient effect
/// carries a colour, a speed and a `reason` string, and the reason here names
/// the app's activity rather than the command - "a command is running" is the
/// whole of what leaves this window.

import { ambient, type AmbientParams } from "@arlen/tauri-plugin-shell";

/// How long a command runs before it is worth saying anything about. See the
/// header: an invented number, deliberately, with its reasoning written down.
export const LONG_RUNNING_MS = 5000;

/// The effect for a command that is still going.
///
/// Accent rather than warning: nothing is wrong, something is taking a while.
/// Slow, because the point is to be noticeable without pulling the eye. Low
/// intensity for the same reason - the cap is 0.5 and this is well under it,
/// since a wash you have to look for is the correct strength for "still going".
///
/// No `auto_clear_ms`: a command that runs for an hour should pulse for an hour,
/// and the clear comes from the command ending. An auto-clear here would say the
/// work had finished when it had not, which is the one thing this must never do.
export function runningEffect(): AmbientParams {
  return {
    effect: "pulse",
    color: "accent",
    intensity: 0.12,
    speed: "slow",
    reason: "a command is running",
  };
}

/// Drives the effect across one window's command boundaries.
///
/// The clock and the publisher are injected so the rule can be tested without a
/// shell to publish into or five real seconds to wait.
export interface AmbientDriver {
  /// A command started. Arms the pulse; nothing is published yet.
  started(): void;
  /// A command ended. Disarms, and takes down anything already showing.
  ended(): void;
  /// Give up the timer and the effect - the window is going away.
  dispose(): void;
}

/// Build a driver over a publisher and a timer.
///
/// `publish` is called with the effect to show, or null to take it down; the
/// caller decides what best-effort means. `setTimer`/`clearTimer` are injected
/// for the same reason - a test drives them directly rather than waiting.
export function commandAmbient(
  publish: (params: AmbientParams | null) => void,
  setTimer: (fn: () => void, ms: number) => number,
  clearTimer: (handle: number) => void,
): AmbientDriver {
  let handle: number | null = null;
  let showing = false;

  const disarm = () => {
    if (handle !== null) {
      clearTimer(handle);
      handle = null;
    }
  };

  return {
    started() {
      // A second exec-start without an end (a nested shell, a mark the engine
      // and the grid both saw) restarts the wait rather than stacking timers.
      disarm();
      handle = setTimer(() => {
        handle = null;
        showing = true;
        publish(runningEffect());
      }, LONG_RUNNING_MS);
    },
    ended() {
      disarm();
      // Only take it down if it went up. A short command that never reached the
      // threshold must not clear an effect another app is showing - the shell
      // keeps one effect per app, and a clear from this app is scoped to this
      // app, but publishing one for a command nobody saw is still a message
      // about nothing.
      if (showing) {
        showing = false;
        publish(null);
      }
    },
    dispose() {
      this.ended();
    },
  };
}

/// The publisher for a real window: best-effort, like every other publish in
/// this tree. Under vite there is no relay and the call rejects, and a terminal
/// that cannot tint the desktop is still a terminal.
export function publishAmbient(params: AmbientParams | null): void {
  void (async () => {
    try {
      if (params) await ambient.set(params);
      else await ambient.clear();
    } catch {
      // No relay: the desktop stays as it was.
    }
  })();
}
