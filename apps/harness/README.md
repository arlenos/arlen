# harness

The Arlen AI app: the graphical place to talk to the AI and to see what it did. It is the full GUI counterpart to the terminal CLI harness. Two surfaces share one window.

Conversation is the query side. You ask, the `ai-daemon` answers, and every tool or graph call it made on the way is shown inline as a collapsible card, so nothing the assistant did is hidden. You can attach files to a turn with an `@`-mention picker, and the history rail holds several conversations you can switch between. Each turn is answered on its own right now; the daemon query path carries no conversation memory yet, and the UI says so rather than pretending otherwise.

The agent surface is the read-only review side. It shows the activity timeline (trigger, behaviour, predict, gate decision, act, audit outcome, newest first), read from the tamper-evident audit ledger, with a filter column for type, outcome and time. It also shows behaviour status and the acting posture (whether the agent only suggests or actually writes), and it is where per-item undo will trigger the agent's compensation once that surface lands.

It is not the autonomous trigger (the Event Bus is) and not the config surface (that is Settings, AI page). This app is where you use and observe the AI, not where you turn it on.

## Stack

Tauri 2, SvelteKit, Svelte 5 and `@arlen/ui-kit`. Built on the Arlen design system with no forked components, so it stays consistent with Settings and the shell. It uses the chat archetype for Conversation and the dashboard archetype for the agent view (see `design-system.md` §5.3).

It talks to the `ai-daemon` for the query path and to `ai-agent` over D-Bus (`org.arlen.AIAgent1`) for agent state and, later, undo. The activity timeline comes from the audit ledger through `audit-proto`; it polls the ledger on open and refresh, and a live Event Bus stream is a later step.

## Spec / ground truth

Read before building:

- `docs/architecture/ai-app.md`: this app's plan, the two surfaces, the shell layout decision (§2.0), the undo loop, the GUI-client survey and the build sequence (A1 through A8).
- `docs/architecture/design-system.md`: the UI canon and the page archetypes.
- `docs/architecture/ai-agent-design.md`: the AI layer and the pull-not-push interaction model.

Those docs live in the private `arlenos/arlen-docs` repo, cloned into `docs/`.

## Status

Pre-alpha, and the active UI strand. Conversation works against the daemon: independent turns, tool-call cards, `@`-mention attachments and a multi-session history rail (in memory for the run). The capability bar and the agent posture banner read from `ai.toml`. The activity timeline and its filters read the audit ledger. Next: persisting sessions to disk so they survive a restart, then per-item undo, which waits on the agent exposing its compensation over D-Bus.
