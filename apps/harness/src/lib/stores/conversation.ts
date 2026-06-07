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

/// A file the user attached to a turn via the composer's `@`-mention picker
/// (ai-app.md §2.1). The text is read and capped backend-side; it is prepended
/// to the prompt the daemon sees, while the user bubble shows only the typed
/// message plus the attachment names.
export interface MentionContent {
  path: string;
  name: string;
  content: string;
  truncated: boolean;
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
  /// Names of files the user attached to this turn (user turns only), shown
  /// as chips on the bubble so the transcript records what was supplemented.
  mentions?: string[];
}

/// Build the prompt actually sent to the daemon: the attached files as a
/// labelled, fenced context block, then the user's message. Each file is
/// wrapped so the model can tell where one ends and the message begins. The
/// user's typed text always comes last so it reads as the live instruction.
function buildPrompt(text: string, mentions: MentionContent[]): string {
  if (mentions.length === 0) return text;
  const blocks = mentions
    .map((m) => {
      const trunc = m.truncated ? " (truncated)" : "";
      return `--- ${m.path}${trunc} ---\n${m.content}`;
    })
    .join("\n\n");
  return `Referenced files:\n${blocks}\n\n${text}`;
}

let nextId = 0;

export const messages = writable<Message[]>([]);
/// True while a turn is in flight; the composer disables itself.
export const busy = writable(false);

/// Submit a prompt and resolve when the turn settles. Pushes the user
/// message and a pending assistant placeholder synchronously, then fills
/// the placeholder with the answer or replaces it with an error.
export async function send(prompt: string, mentions: MentionContent[] = []): Promise<void> {
  const text = prompt.trim();
  // A turn with no typed text but attached files is still meaningful; only an
  // entirely empty submission is dropped.
  if (!text && mentions.length === 0) return;

  const names = mentions.map((m) => m.name);
  messages.update((m) => [
    ...m,
    { id: nextId++, role: "user", text, mentions: names.length ? names : undefined },
  ]);
  const pendingId = nextId++;
  messages.update((m) => [...m, { id: pendingId, role: "assistant", text: "", pending: true }]);
  busy.set(true);

  try {
    const reply = await invoke<{
      answer: string;
      toolCalls: ToolCall[];
      traceUnavailable: boolean;
    }>("ai_query", { prompt: buildPrompt(text, mentions) });
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
