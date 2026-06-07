/// Conversation state for the query surface (ai-app.md §2.1).
///
/// A2 MVP: a transcript of turns backed by the `ai_query` Tauri command
/// (submit → poll → answer). Each turn is independent — only the current
/// prompt is sent; the daemon query path carries no conversation memory
/// yet, so the UI says so and prior turns are not threaded in. One turn
/// is in flight at a time (`busy`). Tool-call parts, citations, and
/// streaming come later; here a message is plain text plus a pending flag.
/// A3 adds the visible tool calls the daemon made while answering.
import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// Who produced a message. `error` is a turn that failed (daemon down,
/// disabled, query error) — rendered distinctly, never as an answer.
export type Role = "user" | "assistant" | "error";

/// One tool call the daemon made while answering (A3). Mirrors the
/// backend `ToolCall`; rendered as a collapsible card so nothing the
/// assistant did is hidden.
export interface ToolCall {
  server: string;
  tool: string;
  arguments: string;
  result: string;
}

export interface Message {
  id: number;
  role: Role;
  text: string;
  /// Assistant placeholder while the daemon is still working.
  pending?: boolean;
  /// Tool calls made while answering (assistant turns only). Empty when
  /// the query took the direct path (and the trace confirmed it).
  toolCalls?: ToolCall[];
  /// True when the tool trace could not be retrieved (distinct from an
  /// empty trace), so the UI says so rather than implying no tools ran.
  traceUnavailable?: boolean;
}

let nextId = 0;

export const messages = writable<Message[]>([]);
/// True while a turn is in flight; the composer disables itself.
export const busy = writable(false);

/// Submit a prompt and resolve when the turn settles. Pushes the user
/// message and a pending assistant placeholder synchronously, then fills
/// the placeholder with the answer or replaces it with an error.
export async function send(prompt: string): Promise<void> {
  const text = prompt.trim();
  if (!text) return;

  messages.update((m) => [...m, { id: nextId++, role: "user", text }]);
  const pendingId = nextId++;
  messages.update((m) => [...m, { id: pendingId, role: "assistant", text: "", pending: true }]);
  busy.set(true);

  try {
    const reply = await invoke<{
      answer: string;
      toolCalls: ToolCall[];
      traceUnavailable: boolean;
    }>("ai_query", { prompt: text });
    messages.update((m) =>
      m.map((msg) =>
        msg.id === pendingId
          ? {
              ...msg,
              text: reply.answer,
              pending: false,
              toolCalls: reply.toolCalls,
              traceUnavailable: reply.traceUnavailable,
            }
          : msg,
      ),
    );
  } catch (e) {
    messages.update((m) =>
      m.map((msg) =>
        msg.id === pendingId
          ? { id: msg.id, role: "error" as const, text: String(e), pending: false }
          : msg,
      ),
    );
  } finally {
    busy.set(false);
  }
}

/// Clear the conversation (new chat).
export function reset(): void {
  messages.set([]);
}
