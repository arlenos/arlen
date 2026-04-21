/// Tauri `invoke()` with a hard timeout.
///
/// Tauri v2 has no native timeout on `invoke()` — if the backend is
/// hung, the returned promise never resolves, which on an init-path
/// (shell startup) freezes the shell indefinitely. This wrapper races
/// the invoke against a `setTimeout` and rejects with a predictable
/// error on timeout so callers can fall back gracefully.
///
/// Typical usage:
///
/// ```ts
/// const data = await invokeWithTimeout<MyShape>("get_thing", {}, 2000)
///   .catch((e) => {
///     console.warn("backend slow, using defaults:", e);
///     return DEFAULTS;
///   });
/// ```
///
/// Keep the timeout generous enough for slow machines but short enough
/// that the shell stays interactive even if the backend is broken.
/// 2-3 seconds is the usual sweet spot for non-critical reads.

import { invoke, type InvokeArgs } from "@tauri-apps/api/core";

export class InvokeTimeoutError extends Error {
    constructor(public readonly command: string, public readonly ms: number) {
        super(`Tauri command "${command}" timed out after ${ms}ms`);
        this.name = "InvokeTimeoutError";
    }
}

/// Invoke a Tauri command with a hard timeout.
///
/// The backend work may still continue in Rust after the timeout fires
/// — we just stop waiting for the result. Callers should design the
/// command to be idempotent-ish so that a timed-out + succeeded call
/// doesn't corrupt state.
export function invokeWithTimeout<T>(
    cmd: string,
    args?: InvokeArgs,
    timeoutMs: number = 2000,
): Promise<T> {
    return new Promise<T>((resolve, reject) => {
        let settled = false;
        const timer = setTimeout(() => {
            if (settled) return;
            settled = true;
            reject(new InvokeTimeoutError(cmd, timeoutMs));
        }, timeoutMs);

        invoke<T>(cmd, args).then(
            (value) => {
                if (settled) return;
                settled = true;
                clearTimeout(timer);
                resolve(value);
            },
            (err) => {
                if (settled) return;
                settled = true;
                clearTimeout(timer);
                reject(err);
            },
        );
    });
}
