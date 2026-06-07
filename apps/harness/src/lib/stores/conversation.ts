/// Conversation state for the query surface (ai-app.md §2.1).
///
/// Each turn is independent against the `ai_query` path (submit → poll →
/// answer); the daemon carries no conversation memory yet, so prior turns are
/// not threaded into the prompt and the UI says so. One turn is in flight at a
/// time (`busy`).
///
/// A8 inc 1: the surface now holds **multiple sessions** (the history rail), not
/// one transcript. This increment keeps them in memory for the run; disk
/// persistence (resumable across restarts) is the next sub-increment, at which
/// point the store decision in `ai-app.md` §8 is made. The page renders the
/// active session through the derived `messages` view, so nothing downstream
/// changed shape.
import { writable, derived, get } from "svelte/store";
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

/// One conversation: an ordered transcript plus a display title and creation
/// time (newest sessions sort first in the rail).
export interface Session {
  id: string;
  title: string;
  messages: Message[];
  createdAt: number;
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

let nextMsgId = 0;

/// Most recent conversations kept on disk. Sessions are newest-first, so the
/// persisted store is capped to this many and older ones fall off.
const MAX_SESSIONS = 100;

/// All conversations this run, newest first.
export const sessions = writable<Session[]>([]);
/// The conversation currently shown; `null` before the first one exists.
export const activeSessionId = writable<string | null>(null);
/// True while a turn is in flight; the composer disables itself.
export const busy = writable(false);

/// The active session's transcript, as a read-only view the page renders. Turns
/// are appended through `send`, never by mutating this directly.
export const messages = derived(
  [sessions, activeSessionId],
  ([$sessions, $id]) => $sessions.find((s) => s.id === $id)?.messages ?? [],
);

let initialized = false;

/// Load persisted sessions on startup and keep the store mirrored to disk.
/// Sessions are in-memory until this runs; call it once when the app mounts.
/// Loading and saving both fail soft, so a persistence problem never breaks the
/// conversation, the rail just falls back to in-memory.
export async function initSessions(): Promise<void> {
  if (initialized) return;
  initialized = true;

  try {
    const stored = await invoke<Session[]>("harness_sessions_load");
    if (Array.isArray(stored) && stored.length > 0) {
      sessions.set(stored);
      activeSessionId.set(stored[0]?.id ?? null);
      // Continue message ids past the highest loaded one, so a new turn in a
      // restored session never collides with an existing keyed message.
      const maxId = Math.max(
        -1,
        ...stored.flatMap((s) => s.messages.map((m) => m.id)),
      );
      nextMsgId = maxId + 1;
    }
  } catch {
    // No persisted history; start fresh.
  }

  // Mirror changes to disk, debounced. Drop in-flight (pending) turns and empty
  // sessions so the stored history stays clean and never freezes a "thinking"
  // placeholder.
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  sessions.subscribe((list) => {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      const clean = list
        .map((s) => ({ ...s, messages: s.messages.filter((m) => !m.pending) }))
        .filter((s) => s.messages.length > 0)
        .slice(0, MAX_SESSIONS);
      invoke("harness_sessions_save", { sessions: clean }).catch(() => {});
    }, 500);
  });
}

const DEFAULT_TITLE = "New conversation";

/// A session's title is the first non-empty user message, truncated; until then
/// it is the placeholder, so the rail reads meaningfully as soon as one is sent.
function titleFrom(msgs: Message[]): string {
  const firstUser = msgs.find((m) => m.role === "user" && m.text.trim().length > 0);
  if (!firstUser) return DEFAULT_TITLE;
  const t = firstUser.text.trim();
  return t.length > 48 ? `${t.slice(0, 48)}…` : t;
}

/// Create a new, empty conversation and make it active.
export function newSession(): string {
  // A UUID, not a per-run counter, so a session restored from a previous run
  // can never collide with a new one.
  const id = crypto.randomUUID();
  sessions.update((list) => [
    { id, title: DEFAULT_TITLE, messages: [], createdAt: Date.now() },
    ...list,
  ]);
  activeSessionId.set(id);
  return id;
}

/// Switch to an existing conversation.
export function selectSession(id: string): void {
  activeSessionId.set(id);
}

/// "New chat" affordance: start a fresh conversation.
export function reset(): void {
  newSession();
}

/// The active session id, creating one lazily so the first message does not
/// need an explicit "New chat" first.
function ensureActive(): string {
  const id = get(activeSessionId);
  if (id && get(sessions).some((s) => s.id === id)) return id;
  return newSession();
}

/// Apply `fn` to one session's messages and refresh its (still-default) title.
function updateSession(id: string, fn: (msgs: Message[]) => Message[]): void {
  sessions.update((list) =>
    list.map((s) => {
      if (s.id !== id) return s;
      const msgs = fn(s.messages);
      const title = s.title === DEFAULT_TITLE ? titleFrom(msgs) : s.title;
      return { ...s, messages: msgs, title };
    }),
  );
}

/// Submit a prompt and resolve when the turn settles. Pushes the user message
/// and a pending assistant placeholder synchronously into the active session,
/// then fills the placeholder with the answer or replaces it with an error. The
/// turn targets the session it was asked in even if the user switches mid-flight.
export async function send(prompt: string, mentions: MentionContent[] = []): Promise<void> {
  const text = prompt.trim();
  // A turn with no typed text but attached files is still meaningful; only an
  // entirely empty submission is dropped.
  if (!text && mentions.length === 0) return;

  const id = ensureActive();
  const names = mentions.map((m) => m.name);
  updateSession(id, (m) => [
    ...m,
    { id: nextMsgId++, role: "user", text, mentions: names.length ? names : undefined },
  ]);
  const pendingId = nextMsgId++;
  updateSession(id, (m) => [...m, { id: pendingId, role: "assistant", text: "", pending: true }]);
  busy.set(true);

  try {
    const reply = await invoke<{
      answer: string;
      toolCalls: ToolCall[];
      traceUnavailable: boolean;
    }>("ai_query", { prompt: buildPrompt(text, mentions) });
    updateSession(id, (m) =>
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
    updateSession(id, (m) =>
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
