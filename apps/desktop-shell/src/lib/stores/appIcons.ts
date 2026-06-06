import { invoke } from "@tauri-apps/api/core";

/** Cache of app_id -> data URL (or null if not found). */
const cache = new Map<string, string | null>();

/** Pending resolution promises to avoid duplicate invoke calls. */
const pending = new Map<string, Promise<string | null>>();

/**
 * Resolves a freedesktop icon for the given app_id.
 * Returns a base64 data URL suitable for <img src>, or null if not found.
 * Results are cached for the session lifetime.
 */
export async function resolveAppIcon(appId: string): Promise<string | null> {
    if (cache.has(appId)) return cache.get(appId)!;
    if (pending.has(appId)) return pending.get(appId)!;

    const promise = invoke<string | null>("resolve_app_icon", { appId })
        .then((dataUrl) => {
            cache.set(appId, dataUrl);
            pending.delete(appId);
            return dataUrl;
        })
        .catch(() => {
            cache.set(appId, null);
            pending.delete(appId);
            return null;
        });

    pending.set(appId, promise);
    return promise;
}
