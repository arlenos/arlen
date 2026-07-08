/// The voice HUD (shell-voice-plan.md): the thin push-to-talk surface. Invoke-then-
/// listen, never Alexa-always-on - the mic is on exactly while you hold the key, the
/// speech runs ON-DEVICE, and every session lands in the audit ledger. The HUD shows
/// the moment: listening -> your words -> (for a question) the agent thinking -> its
/// short reply.
///
/// Mock-vs-live: fixture-backed. The compositor push-to-talk shortcut, the on-device
/// STT stream (fills `transcript`), the agent round-trip (fills `reply`), and the mic
/// privacy-dot + audit wiring are coder seams; under vite the store drives the phases.

import { writable } from "svelte/store";

/// Where in the round-trip we are.
export type VoicePhase = "idle" | "listening" | "thinking" | "replying";

/// The HUD state.
export interface VoiceState {
  phase: VoicePhase;
  /// What you said, streamed from the on-device recogniser.
  transcript: string;
  /// The agent's reply (converse mode only).
  reply: string;
  /// True when talking WITH the agent (vs plain dictation).
  converse: boolean;
}

export const voice = writable<VoiceState>({ phase: "idle", transcript: "", reply: "", converse: false });

/// Begin listening (the push-to-talk key went down). Live: the compositor shortcut
/// + the STT stream drive this.
export function startListening(converse = true): void {
  voice.set({ phase: "listening", transcript: "", reply: "", converse });
}

/// Close the HUD.
export function dismiss(): void {
  voice.set({ phase: "idle", transcript: "", reply: "", converse: false });
}

// Dev-only: walk the phases so the surface renders each state (the screenshot loop).
const DEMO = "What's on my calendar tomorrow?";
const REPLY = "Three things: the 9:30 standup, lunch with Alex at noon and the review at 4.";
let timers: ReturnType<typeof setTimeout>[] = [];

/// Dev-only: drive listening -> streamed transcript -> thinking -> reply.
export function cycleMock(): void {
  timers.forEach(clearTimeout);
  timers = [];
  voice.set({ phase: "listening", transcript: "", reply: "", converse: true });
  const words = DEMO.split(" ");
  words.forEach((_, i) => {
    timers.push(
      setTimeout(() => voice.update((v) => ({ ...v, transcript: words.slice(0, i + 1).join(" ") })), 220 * (i + 1)),
    );
  });
  const done = 220 * (words.length + 1);
  timers.push(setTimeout(() => voice.update((v) => ({ ...v, phase: "thinking" })), done));
  timers.push(setTimeout(() => voice.update((v) => ({ ...v, phase: "replying", reply: REPLY })), done + 900));
}
