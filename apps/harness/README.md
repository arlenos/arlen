# app-harness

The Arlen **AI Harness App** - the graphical agent/conversation and
observability surface for the AI layer. The full GUI door to the AI (the
graphical counterpart to the terminal CLI harness): multi-turn conversation
with streaming and visible tool calls, plus the read-only activity timeline
(trigger → behaviour → predict → gate → act → audit) where the user reviews
what the silent curator did and can undo individual curated actions.

It is **not** the autonomous trigger (the Event Bus is) and **not** the config
surface (that is Settings → AI). This app is where you *use and observe* the AI.

## Stack

Tauri 2 + SvelteKit + Svelte 5 + `@arlen/ui-kit`. Built entirely on the
Arlen Design System (single component source, no forked primitives) - a
greenfield exemplar of the unified UI.

Talks to: `ai-daemon` (query/chat, streaming), `ai-agent` over D-Bus
(`org.arlen.AIAgent1`; agent state, undo/compensation), the audit ledger
(activity timeline, via `audit-proto`), and the Event Bus (live activity).

## Spec / ground truth

Read before building:

- `docs/architecture/ai-app.md` - this app's plan (surfaces, transparency
  principles, the undo/compensation loop, the open-source GUI-client survey,
  build sequence A1-A6).
- `docs/architecture/design-system.md` - the UI canon this app is built on.
- `docs/architecture/ai-agent-design.md` - the AI layer, interaction model
  (silent curator + pull), capability model.

(Those docs live in the shared `arlenos/docs/` folder.)

## Status

Scaffold pending (build phase A1). This repo currently holds only the bootstrap.
