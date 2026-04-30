/**
 * @lunaris/tauri-plugin-clipboard
 *
 * TypeScript bindings for the Lunaris clipboard plugin. Wraps the
 * shell-broker IPC at `$XDG_RUNTIME_DIR/lunaris/clipboard.sock`,
 * giving first-party apps a sandbox-safe replacement for the raw
 * Wayland `wl_data_device` interface.
 *
 * # Permissions
 *
 * Each call requires the matching scope in the app's
 * permission profile:
 *
 * | Call         | Scope            |
 * |--------------|------------------|
 * | `write()`    | `clipboard.write`|
 * | `read()`     | `clipboard.read` |
 * | `subscribe()`| `clipboard.read` |
 * | `history()`  | `clipboard.history` |
 *
 * Sensitive content additionally requires `clipboard.read_sensitive`
 * for `read()` and `subscribe()` to receive the bytes; without it
 * the entry is delivered with `content === undefined`.
 *
 * # Example
 *
 * ```typescript
 * import { write, subscribe } from '@lunaris/tauri-plugin-clipboard';
 *
 * await write({ content: new TextEncoder().encode('hello'), mime: 'text/plain' });
 *
 * await subscribe();
 * import { listen } from '@tauri-apps/api/event';
 * await listen<ClipboardEntry>('lunaris://clipboard-changed', (e) => {
 *   console.log('clipboard changed:', e.payload);
 * });
 * ```
 */

import { invoke } from "@tauri-apps/api/core";

// ── Types ────────────────────────────────────────────────────────

export type ClipboardLabel = "normal" | "sensitive";

export interface ClipboardEntry {
  /** Stable id assigned by the shell. */
  id: string;
  /** Bytes of the content, or `undefined` if the caller lacks
   *  read permission for this entry. */
  content?: Uint8Array;
  /** MIME type of the content. Phase 1 supports `text/plain`. */
  mime: string;
  label: ClipboardLabel;
  /** Unix milliseconds at capture time. */
  timestampMs: number;
  /** App id inferred from the focused window at capture time;
   *  empty if the shell could not determine a source. */
  sourceAppId: string;
}

export interface WriteParams {
  content: Uint8Array;
  mime: string;
  /** Defaults to `"normal"`. */
  label?: ClipboardLabel;
}

// ── Internal helpers ─────────────────────────────────────────────

interface RawClipboardEntry {
  id: string;
  content?: number[];
  mime: string;
  label: ClipboardLabel;
  timestampMs: number;
  sourceAppId: string;
}

function decodeEntry(raw: RawClipboardEntry): ClipboardEntry {
  return {
    id: raw.id,
    content: raw.content === undefined ? undefined : new Uint8Array(raw.content),
    mime: raw.mime,
    label: raw.label,
    timestampMs: raw.timestampMs,
    sourceAppId: raw.sourceAppId,
  };
}

function encodeWrite(params: WriteParams): {
  content: number[];
  mime: string;
  label: ClipboardLabel;
} {
  return {
    content: Array.from(params.content),
    mime: params.mime,
    label: params.label ?? "normal",
  };
}

// ── Public API ───────────────────────────────────────────────────

/**
 * Place content on the clipboard. Throws on permission denial,
 * oversized content, or unsupported MIME.
 */
export async function write(params: WriteParams): Promise<void> {
  await invoke("plugin:lunaris-clipboard|write", {
    params: encodeWrite(params),
  });
}

/**
 * Read the current clipboard content. Returns `null` if the
 * clipboard is empty.
 */
export async function read(): Promise<ClipboardEntry | null> {
  const raw = await invoke<RawClipboardEntry | null>(
    "plugin:lunaris-clipboard|read"
  );
  return raw === null ? null : decodeEntry(raw);
}

/**
 * Return the clipboard history, newest first. `limit` defaults
 * to 50 server-side. Requires `clipboard.history` scope.
 */
export async function history(limit?: number): Promise<ClipboardEntry[]> {
  const raw = await invoke<RawClipboardEntry[]>(
    "plugin:lunaris-clipboard|history",
    { limit }
  );
  return raw.map(decodeEntry);
}

/**
 * Subscribe to clipboard changes. Subsequent updates are emitted
 * as `lunaris://clipboard-changed` Tauri events with a
 * `ClipboardEntry` payload. Use `@tauri-apps/api/event` `listen`
 * to receive them.
 *
 * Calling twice without `unsubscribe()` rejects with
 * `AlreadySubscribed`.
 */
export async function subscribe(): Promise<void> {
  await invoke("plugin:lunaris-clipboard|subscribe");
}

/** Cancel an active subscription. */
export async function unsubscribe(): Promise<void> {
  await invoke("plugin:lunaris-clipboard|unsubscribe");
}
