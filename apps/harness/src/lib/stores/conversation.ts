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
import { planRegenerate } from "$lib/regenerate";
import { planEdit } from "$lib/edit";
import { planDelete } from "$lib/delete";
import { planFork } from "$lib/fork";
import { togglePinned } from "$lib/bookmark";
import { clearDraft } from "$lib/stores/drafts";
import { parseConversationEnvelope } from "$lib/export";
import { sortSessions } from "$lib/pin-session";
import type { Artifact } from "$lib/components/artifact/types";

/// Who produced a message. `error` is a turn that failed (daemon down,
/// disabled, query error) — rendered distinctly, never as an answer.
export type Role = "user" | "assistant" | "error";

/// One tool call the daemon made while answering (A3). Mirrors the
/// backend `ToolCall`; rendered as a collapsible card so nothing the
/// assistant did is hidden.
/// Whether a recorded tool call succeeded (harness-redesign emit seam 1,
/// mirrors the daemon's `ToolStatus`). `running` is the in-flight state shown
/// before the trace entry lands; the trace itself only carries `done` /
/// `failed`. Drives the tool-call card's status glyph.
export type ToolStatus = "running" | "done" | "failed";

export interface ToolCall {
  server: string;
  tool: string;
  arguments: string;
  result: string;
  /// Outcome from the daemon trace. Absent only on conversations persisted
  /// before this field existed, which the card renders without a glyph.
  status?: ToolStatus;
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
  /// True when the user bookmarked this message, so it can be revisited from a
  /// bookmarks view. Absent (not `false`) when unmarked, to keep the stored
  /// record minimal.
  pinned?: boolean;
  /// Artifacts the assistant produced this turn (the coder's command fills
  /// this). Each renders inline or as a card that opens the right pane, decided
  /// by kind + size in `placement`.
  artifacts?: Artifact[];
}

/// One conversation: an ordered transcript plus a display title and creation
/// time (newest sessions sort first in the rail).
export interface Session {
  id: string;
  title: string;
  messages: Message[];
  createdAt: number;
  /// True when the user pinned this conversation to the top of the rail. Absent
  /// (not `false`) when unpinned, to keep the stored record minimal.
  pinned?: boolean;
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

/// The sessions in rail order: pinned conversations first (see `sortSessions`),
/// otherwise the store's newest-first order. The rail reads this so pinning
/// takes effect without the store itself reordering (which would complicate
/// id-recovery and the save mirror).
export const orderedSessions = derived(sessions, ($sessions) => sortSessions($sessions));
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

/// The active session's title, for the titlebar. Empty when no session is
/// active (the titlebar then falls back to the generic surface name).
export const activeTitle = derived(
  [sessions, activeSessionId],
  ([$sessions, $id]) => $sessions.find((s) => s.id === $id)?.title ?? "",
);

let initialized = false;

/// Coerce one record loaded from the schema-agnostic sessions file into a
/// well-formed [`Session`], or return `null` to drop it. Unknown roles,
/// non-string text, non-array messages, and missing ids are all handled, so a
/// stale or corrupted record can never enter the store and break id-recovery,
/// the save mirror, or the rail. A pending flag is never restored (placeholders
/// are dropped before save; a loaded "pending" turn would otherwise freeze).
export function sanitizeSession(raw: unknown): Session | null {
  if (typeof raw !== "object" || raw === null) return null;
  const r = raw as Record<string, unknown>;
  const id = typeof r.id === "string" && r.id.length > 0 ? r.id : null;
  if (id === null) return null;
  const title = typeof r.title === "string" ? r.title : DEFAULT_TITLE;
  const createdAt = typeof r.createdAt === "number" ? r.createdAt : 0;
  const rawMsgs = Array.isArray(r.messages) ? r.messages : [];
  const messages: Message[] = [];
  for (const item of rawMsgs) {
    if (typeof item !== "object" || item === null) continue;
    const m = item as Record<string, unknown>;
    if (typeof m.id !== "number") continue;
    if (m.role !== "user" && m.role !== "assistant" && m.role !== "error") continue;
    const msg: Message = {
      id: m.id,
      role: m.role as Role,
      text: typeof m.text === "string" ? m.text : "",
    };
    if (Array.isArray(m.mentions)) {
      const names = m.mentions.filter((x): x is string => typeof x === "string");
      if (names.length > 0) msg.mentions = names;
    }
    if (Array.isArray(m.toolCalls)) msg.toolCalls = m.toolCalls as ToolCall[];
    if (typeof m.traceUnavailable === "boolean") msg.traceUnavailable = m.traceUnavailable;
    // Only a true bookmark is restored; an absent or false flag stays unset.
    if (m.pinned === true) msg.pinned = true;
    messages.push(msg);
  }
  const session: Session = { id, title, createdAt, messages };
  // Only a true pin is restored; an absent or non-true flag stays unset.
  if (r.pinned === true) session.pinned = true;
  return session;
}

/// Load persisted sessions on startup and keep the store mirrored to disk.
/// Sessions are in-memory until this runs; call it once when the app mounts.
/// Loading and saving both fail soft, so a persistence problem never breaks the
/// conversation, the rail just falls back to in-memory.
export async function initSessions(): Promise<void> {
  if (initialized) return;
  initialized = true;

  try {
    const stored = await invoke<unknown[]>("harness_sessions_load");
    if (Array.isArray(stored) && stored.length > 0) {
      // The sessions file is schema-agnostic JSON owned by the frontend, so a
      // record may predate the current shape or be partially corrupted.
      // Sanitize before anything touches it: the store, the id-recovery below,
      // and the save mirror all assume well-formed sessions with array
      // messages, so an unsanitized `messages: null` (or similar) would throw
      // here and keep destabilizing the rail.
      const clean = stored
        .map(sanitizeSession)
        .filter((s): s is Session => s !== null);
      if (clean.length > 0) {
        sessions.set(clean);
        activeSessionId.set(clean[0].id);
        // Continue message ids past the highest loaded one, so a new turn in a
        // restored session never collides with an existing keyed message.
        const maxId = Math.max(
          -1,
          ...clean.flatMap((s) => s.messages.map((m) => m.id)),
        );
        nextMsgId = maxId + 1;
      }
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

/// Create a new, empty conversation and make it active. Idempotent on an
/// already-empty active chat: asking for a new chat while sitting on a blank
/// one focuses that one instead of stacking another blank.
export function newSession(): string {
  const activeId = get(activeSessionId);
  const active = activeId
    ? get(sessions).find((s) => s.id === activeId)
    : undefined;
  if (active && active.messages.length === 0) return active.id;

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

/// Toggle whether a conversation is pinned to the top of the rail (see
/// `sortSessions` / `orderedSessions`). Pinning sets `pinned: true`; unpinning
/// removes the flag entirely, so an unpinned session carries no `pinned` key.
export function togglePinSession(id: string): void {
  sessions.update((list) =>
    list.map((s) => {
      if (s.id !== id) return s;
      if (s.pinned) {
        const next = { ...s };
        delete next.pinned;
        return next;
      }
      return { ...s, pinned: true };
    }),
  );
}

/// Rename a conversation. A non-empty title is taken verbatim (trimmed) and
/// becomes sticky: because it is no longer the placeholder, `updateSession`
/// stops re-deriving the title from the first message, so the user's name wins.
/// An empty title resets to the auto-derived title (the first message, or the
/// placeholder when there is none), so clearing a custom name hands control back
/// to the automatic one.
export function renameSession(id: string, title: string): void {
  const next = title.trim();
  sessions.update((list) =>
    list.map((s) =>
      s.id === id
        ? { ...s, title: next.length > 0 ? next : titleFrom(s.messages) }
        : s,
    ),
  );
}

/// Delete a conversation. If it was the active one, fall back to the newest
/// remaining session (or none). The debounced disk save then drops it.
export function deleteSession(id: string): void {
  sessions.update((list) => list.filter((s) => s.id !== id));
  if (get(activeSessionId) === id) {
    activeSessionId.set(get(sessions)[0]?.id ?? null);
  }
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
  // The prompt is now sent, so its saved composer draft is spent.
  clearDraft(id);
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

/// Regenerate the active conversation's last response: drop it and re-ask the
/// same question. A no-op when the conversation cannot be regenerated (see
/// `planRegenerate`), so a caller can wire it to a button gated on the same
/// check. The fresh answer replaces the dropped one in place; the turn targets
/// the session it was asked in even if the user switches mid-flight.
export async function regenerate(): Promise<void> {
  const id = get(activeSessionId);
  if (!id) return;
  const session = get(sessions).find((s) => s.id === id);
  if (!session) return;
  const plan = planRegenerate(session.messages);
  if (!plan) return;

  const pendingId = nextMsgId++;
  // Keep the existing transcript and append a placeholder, rather than
  // truncating to `plan.keep` up front. The previous answer (and its tool
  // calls / trace) stay visible and persisted until a replacement actually
  // succeeds, so a daemon error, a hang, or the app closing mid-regenerate
  // never loses it (the disk save only drops the pending placeholder).
  updateSession(id, (m) => [
    ...m,
    { id: pendingId, role: "assistant", text: "", pending: true },
  ]);
  busy.set(true);

  try {
    // The plan only regenerates attachment-free turns, so the prompt is the
    // user's text verbatim (no referenced-files block to rebuild).
    const reply = await invoke<{
      answer: string;
      toolCalls: ToolCall[];
      traceUnavailable: boolean;
    }>("ai_query", { prompt: plan.prompt });
    // Success: atomically swap to the kept prefix plus the fresh answer,
    // dropping the old response and the placeholder in one update.
    updateSession(id, () => [
      ...plan.keep,
      {
        id: pendingId,
        role: "assistant",
        text: reply.answer,
        pending: false,
        toolCalls: reply.toolCalls,
        traceUnavailable: reply.traceUnavailable,
      },
    ]);
  } catch (e) {
    // Failure: keep the old response, turn only the placeholder into an error
    // turn. The previous answer is preserved above it, and the trailing error
    // is itself regeneratable, so the user can retry.
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

/// Edit a prior user message in the active conversation and re-ask it: drop that
/// turn and everything after it, then send the new text. A no-op when the edit
/// is not allowed (see `planEdit`), so a caller can wire it to an inline editor
/// gated on the same check. Like `regenerate`, the original transcript stays
/// visible and persisted until the replacement succeeds: the edited question and
/// a placeholder are appended, the truncation happens atomically only on success,
/// and a mid-flight failure turns just the placeholder into a retryable error
/// without losing the prior conversation. The turn targets the session it was
/// edited in even if the user switches mid-flight.
export async function editAndResend(messageId: number, newText: string): Promise<void> {
  const id = get(activeSessionId);
  if (!id) return;
  const session = get(sessions).find((s) => s.id === id);
  if (!session) return;
  const plan = planEdit(session.messages, messageId, newText);
  if (!plan) return;

  const userId = nextMsgId++;
  const pendingId = nextMsgId++;
  updateSession(id, (m) => [
    ...m,
    { id: userId, role: "user", text: plan.prompt },
    { id: pendingId, role: "assistant", text: "", pending: true },
  ]);
  busy.set(true);

  try {
    // The plan only edits attachment-free turns, so the prompt is the new text
    // verbatim (no referenced-files block to rebuild).
    const reply = await invoke<{
      answer: string;
      toolCalls: ToolCall[];
      traceUnavailable: boolean;
    }>("ai_query", { prompt: plan.prompt });
    // Success: atomically swap to the kept prefix, the edited question, and the
    // fresh answer, dropping the old turns from the edit point onward.
    updateSession(id, () => [
      ...plan.keep,
      { id: userId, role: "user", text: plan.prompt },
      {
        id: pendingId,
        role: "assistant",
        text: reply.answer,
        pending: false,
        toolCalls: reply.toolCalls,
        traceUnavailable: reply.traceUnavailable,
      },
    ]);
  } catch (e) {
    // Failure: keep the original transcript and the edited question, turning
    // only the placeholder into a retryable error. Nothing is truncated, so the
    // prior conversation is never lost on a daemon error or hang.
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

/// Delete a turn from the active conversation: remove the message with `id` and,
/// for a question, the answer that belongs with it (see `planDelete`). A no-op
/// when the delete is not allowed (a turn in flight, or an unknown id). Local
/// and synchronous, so it persists immediately with no daemon round-trip; a
/// caller wires it to a per-message delete control gated on the same check.
export function deleteTurn(messageId: number): void {
  const id = get(activeSessionId);
  if (!id) return;
  const session = get(sessions).find((s) => s.id === id);
  if (!session) return;
  const next = planDelete(session.messages, messageId);
  if (!next) return;
  updateSession(id, () => next);
}

/// Fork the active conversation at a chosen turn into a new session: copy the
/// prefix through that message into a fresh conversation and make it active,
/// leaving the original untouched (see `planFork`). Returns the new session id,
/// or `null` when the fork is not allowed (a turn in flight, or an unknown id).
/// The new session derives its own title from the copied prefix; later turns
/// added to the branch never touch the original.
export function fork(messageId: number): string | null {
  const id = get(activeSessionId);
  if (!id) return null;
  const session = get(sessions).find((s) => s.id === id);
  if (!session) return null;
  const prefix = planFork(session.messages, messageId);
  if (!prefix) return null;

  const newId = crypto.randomUUID();
  sessions.update((list) => [
    { id: newId, title: titleFrom(prefix), messages: prefix, createdAt: Date.now() },
    ...list,
  ]);
  activeSessionId.set(newId);
  return newId;
}

/// Toggle the bookmark on a message in the active conversation (see
/// `togglePinned`). Local and synchronous, so it persists immediately with no
/// daemon round-trip; a caller wires it to a per-message bookmark control.
export function togglePin(messageId: number): void {
  const id = get(activeSessionId);
  if (!id) return;
  updateSession(id, (messages) => togglePinned(messages, messageId));
}

/// Import a conversation from a JSON export envelope (see `conversationToJson`):
/// validate the envelope, sanitise the embedded session, give it a fresh id so
/// it cannot clobber an existing conversation, then add it and make it active.
/// Returns the new session id, or `null` when the input is not a valid envelope
/// or its session does not survive sanitisation. A re-import of a file the user
/// already has therefore creates a distinct copy rather than overwriting.
export function importConversation(json: string): string | null {
  const raw = parseConversationEnvelope(json);
  if (raw === null) return null;
  const session = sanitizeSession(raw);
  if (!session) return null;

  const newId = crypto.randomUUID();
  const imported: Session = { ...session, id: newId };
  sessions.update((list) => [imported, ...list]);
  activeSessionId.set(newId);
  return newId;
}
