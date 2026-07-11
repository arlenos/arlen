<script lang="ts">
  /// Render harness for the agent-actions tray (the live path needs the agent
  /// daemon, absent under vite). Feeds the stores fixture proposals + receipts so
  /// the gate/receipt cards and their placement above the composer can be
  /// screenshot-verified. Not shipped UI - the live chat polls the real commands.
  import { onMount } from "svelte";
  import AgentActions from "$lib/components/chat/AgentActions.svelte";
  import { pendingProposals, completedActions } from "$lib/stores/agentActions";

  onMount(() => {
    pendingProposals.set([
      {
        id: 42,
        behaviour: "auto-tag-by-project",
        tool: "graph.write",
        summary: "Tag 3 files as part of the Arlen project",
        reason: "you edited them together in the last hour",
        effects: ["FILE_PART_OF: 3 files -> Arlen"],
        operands: [],
        change: null,
      },
      {
        id: 43,
        behaviour: "tidy-downloads",
        tool: "fs.move",
        summary: "Move report.pdf into Documents/Projects",
        reason: "it matches the project you are working in",
        effects: ["moved report.pdf -> Documents/Projects"],
        operands: [],
        change: { kind: "rename", summary: "Move report.pdf", from: "~/Downloads/report.pdf", to: "~/Documents/Projects/report.pdf" },
      },
    ]);
    completedActions.set([
      { id: "corr-9", behaviour: "auto-tag-by-project", what: "Tagged notes.md as part of Arlen", change: null },
      { id: "corr-8", behaviour: "tidy-downloads", what: "Moved invoice.pdf into Documents", change: null },
    ]);
  });
</script>

<div class="stage">
  <div class="col">
    <AgentActions />
    <div class="fake-composer">Ask about your files, projects, or activity</div>
  </div>
</div>

<style>
  .stage {
    display: flex;
    align-items: flex-end;
    justify-content: center;
    height: 100%;
    padding: 2rem 1rem;
    background: var(--background);
  }
  .col {
    display: flex;
    flex-direction: column;
    width: 100%;
    max-width: var(--width-thread, 48rem);
  }
  .fake-composer {
    padding: 0.75rem 1rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-card);
    background: var(--card);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-size: var(--text-base);
  }
</style>
