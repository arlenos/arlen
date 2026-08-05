import { invoke } from "@tauri-apps/api/core";

/**
 * Forward everything the webview would only have written to a console nobody
 * can read into the shell's own log, where a boot journal picks it up.
 *
 * On the image there is no devtools to open, so a `console.error` and an
 * unhandled rejection are equally invisible: the shell keeps running, the
 * screen looks fine, and the only evidence of the failure is in a buffer that
 * dies with the process. That is what made the standing consent dialog take
 * several boots to diagnose - the click could have failed at four different
 * places and every one of them looked identical from outside.
 *
 * `frontend_log` already existed but had to be called by hand, so it only
 * covered the one path someone had already suspected. This covers the paths
 * nobody has suspected yet, which are the ones that cost the hours.
 */

const MAX_ARG = 400;
const MAX_LINE = 2000;

/** Render one console argument compactly and without ever throwing. An
 *  unserialisable value must not be the reason a log line goes missing. */
function render(value: unknown): string {
  if (typeof value === "string") return value;
  if (value instanceof Error) {
    return value.stack ? `${value.message}\n${value.stack}` : value.message;
  }
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

function line(args: unknown[]): string {
  return args
    .map((a) => {
      const s = render(a);
      return s.length > MAX_ARG ? `${s.slice(0, MAX_ARG)}...` : s;
    })
    .join(" ")
    .slice(0, MAX_LINE);
}

/**
 * Install the bridge. Returns a disposer that restores the original console
 * methods and removes the window handlers, so an HMR reload does not stack
 * wrappers on top of each other.
 */
export function initConsoleBridge(): () => void {
  const forward = (level: "warn" | "error", args: unknown[]) => {
    // Never let logging fail the thing it is logging about.
    void invoke("frontend_log", { level, msg: line(args) }).catch(() => {});
  };

  const originalWarn = console.warn;
  const originalError = console.error;

  console.warn = (...args: unknown[]) => {
    originalWarn.apply(console, args as []);
    forward("warn", args);
  };
  console.error = (...args: unknown[]) => {
    originalError.apply(console, args as []);
    forward("error", args);
  };

  // A thrown error and a rejected promise never reach console.error on their
  // own, and they are exactly the shape of "the click did nothing".
  const onError = (e: ErrorEvent) => {
    forward("error", [`uncaught: ${e.message}`, `${e.filename}:${e.lineno}`]);
  };
  const onRejection = (e: PromiseRejectionEvent) => {
    forward("error", ["unhandled rejection:", e.reason]);
  };
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);

  return () => {
    console.warn = originalWarn;
    console.error = originalError;
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
  };
}
