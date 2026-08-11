/// The Waypointer quick-ask state (waypointer-ai-prompt.md): Tab flips the
/// launcher into Ask mode, the answer streams inline, Ctrl+J escalates to the
/// harness on the same server-side session. Store-based (not $state) because of
/// the documented IPC-callback re-render caveat in this overlay.
///
/// Mock-vs-live: `waypointer_ask` is live - it calls `org.arlen.AI1.ask`, which
/// runs the `ask` skill on a bounded ephemeral engine and returns one answer.
/// **One answer per ask**: each run dies with its reply, so a follow-up does not
/// remember the previous turn; the session id is this pane's thread, not a
/// server-side one. `ai_capability` and the harness session entry
/// (`open_harness_session`) are still coder seams. Under vite/DEV a fixture
/// streams a canned answer so the pane is drivable; live without a reachable
/// engine the pane says the agent is unreachable.
import { get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// What the capability line renders (the harness `ai_capability` shape).
export interface AskCapability {
  enabled: boolean;
  tier: string;
  actionMode: string;
  provider?: string;
  model?: string;
  executorLive: boolean;
}

/// One exchange in the inline thread.
export interface AskTurn {
  role: "you" | "agent";
  text: string;
}

/// Whether the launcher is in Ask mode (Tab toggles; Esc drops back to search).
export const askMode = writable(false);
/// The inline exchanges of the current session, oldest first.
export const askTurns = writable<AskTurn[]>([]);
/// True while an answer is still arriving.
export const askStreaming = writable(false);
/// The read capability, or null while unknown/unreachable.
export const askCapability = writable<AskCapability | null>(null);
/// True once the capability read settled (so nothing flashes).
export const askCapabilityLoaded = writable(false);
/// True when a live ask failed to reach the agent (never set by the DEV fixture).
export const askUnreachable = writable(false);

const FIXTURE_CAPABILITY: AskCapability = {
  enabled: true,
  tier: "Project",
  actionMode: "Suggest",
  provider: "ollama-default",
  model: "qwen2.5:7b",
  executorLive: true,
};

const FIXTURE_ANSWER =
  "Balanced until the battery reaches 20%, then battery saver takes over. " +
  "Suspend on lid close stays on for both. You set this under Settings > Power " +
  "two weeks ago and have not changed it since.";

let sessionId: string | null = null;
let fixtureTimer: ReturnType<typeof setInterval> | null = null;

/// Read the capability for the line under the input. Live: `ai_capability` in
/// this app's own backend (`src-tauri/src/capability.rs`); DEV falls back to the
/// fixture.
export async function loadAskCapability(): Promise<void> {
  try {
    askCapability.set(await invoke<AskCapability>("ai_capability"));
  } catch {
    askCapability.set(import.meta.env.DEV ? FIXTURE_CAPABILITY : null);
  } finally {
    askCapabilityLoaded.set(true);
  }
}

/// The status line for the two states that speak (off / unreachable); the
/// healthy state renders nothing.
export function capabilitySentence(c: AskCapability | null): string {
  if (c === null) return "The agent isn't reachable right now.";
  return "AI is off. The agent won't read or answer anything.";
}

/// Send a prompt (first ask or follow-up). Live: `waypointer_ask` streams the
/// bounded read-tier answer into the last turn; DEV streams the fixture.
export async function ask(prompt: string): Promise<void> {
  const q = prompt.trim();
  if (!q || get(askStreaming)) return;
  askTurns.update((t) => [...t, { role: "you", text: q }, { role: "agent", text: "" }]);
  askStreaming.set(true);
  askUnreachable.set(false);
  try {
    const answer = await invoke<{ session: string; text: string }>("waypointer_ask", {
      prompt: q,
      session: sessionId,
    });
    sessionId = answer.session;
    appendToAgentTurn(answer.text, true);
  } catch {
    if (import.meta.env.DEV) {
      streamFixture();
    } else {
      askTurns.update((t) => t.slice(0, -2));
      askStreaming.set(false);
      askUnreachable.set(true);
    }
  }
}

function appendToAgentTurn(text: string, done: boolean): void {
  askTurns.update((t) => {
    const next = [...t];
    const last = next[next.length - 1];
    if (last?.role === "agent") next[next.length - 1] = { ...last, text: last.text + text };
    return next;
  });
  if (done) askStreaming.set(false);
}

// DEV only: grow the answer word by word so the streaming state is a real,
// visible state rather than a claim.
function streamFixture(): void {
  const words = FIXTURE_ANSWER.split(" ");
  let i = 0;
  if (fixtureTimer) clearInterval(fixtureTimer);
  fixtureTimer = setInterval(() => {
    if (i >= words.length) {
      if (fixtureTimer) clearInterval(fixtureTimer);
      fixtureTimer = null;
      askStreaming.set(false);
      return;
    }
    appendToAgentTurn((i === 0 ? "" : " ") + words[i], false);
    i += 1;
  }, 40);
}

/// Ctrl+J: open the harness on this session (full fidelity). The session id, not
/// the transcript, travels - both surfaces are thin clients of the same daemon.
export async function escalate(): Promise<void> {
  if (sessionId === null && !import.meta.env.DEV) return;
  try {
    await invoke("open_harness_session", { id: sessionId ?? "dev-session" });
  } catch {
    // Seam unwired: nothing to open under vite.
  }
}

/// Leave Ask mode (Esc). The session persists server-side; only the local pane
/// clears so search comes back clean.
export function resetAsk(): void {
  if (fixtureTimer) clearInterval(fixtureTimer);
  fixtureTimer = null;
  askMode.set(false);
  askTurns.set([]);
  askStreaming.set(false);
  askUnreachable.set(false);
  sessionId = null;
}
