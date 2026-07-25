<script lang="ts">
  /// The inline quick-ask pane (waypointer-ai-prompt.md): shown instead of the
  /// result list while the launcher is in Ask mode. The capability line under
  /// the input, the turns scroller, and the escalation footer. Plain text only -
  /// no rendered LLM HTML in the shell, deliberately (the overlay is the most
  /// privileged surface; markdown here is a separate planner decision).
  import {
    askTurns,
    askStreaming,
    askCapability,
    askCapabilityLoaded,
    askUnreachable,
    capabilitySentence,
  } from "$lib/stores/waypointerAsk";

  const enabled = $derived($askCapability?.enabled ?? false);

  // Keep the newest text in view while the answer grows.
  let scroller = $state<HTMLElement | null>(null);
  $effect(() => {
    void $askTurns;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  });
</script>

<div class="ask">
  {#if $askCapabilityLoaded}
    <p class="ask-cap" class:off={!enabled}>
      <span class="ask-glyph" class:off={!enabled} aria-hidden="true">◆</span>
      {capabilitySentence($askCapability)}
    </p>
  {/if}

  {#if enabled}
    {#if $askTurns.length > 0}
      <div class="ask-turns" bind:this={scroller}>
        {#each $askTurns as turn, i (i)}
          {#if turn.role === "you"}
            <p class="ask-you">{turn.text}</p>
          {:else}
            <p class="ask-answer">{turn.text}{#if $askStreaming && i === $askTurns.length - 1}<span class="ask-caret" aria-hidden="true"></span>{/if}</p>
          {/if}
        {/each}
      </div>
    {/if}
    {#if $askUnreachable}
      <p class="ask-note">The agent isn't reachable right now.</p>
    {/if}
  {/if}

  <div class="ask-footer">
    {#if !enabled}
      <span>Esc back to search</span>
    {:else if $askTurns.length === 0}
      <span>Enter ask</span>
      <span>Esc back to search</span>
    {:else}
      <span>Enter follow-up</span>
      <span>Ctrl+J continue in agent</span>
      <span>Esc back</span>
    {/if}
  </div>
</div>

<style>
  .ask {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* The capability line: the same one-sentence anchor the harness uses, compact. */
  .ask-cap {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    margin: 0;
    padding: 0.5rem 0.9rem 0.35rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ask-glyph {
    color: var(--color-success);
    font-size: 0.55rem;
  }
  .ask-glyph.off {
    color: color-mix(in srgb, var(--color-fg-shell) 35%, transparent);
  }

  /* The turns: a bounded scroller (never flex-grow - the layer-shell overlay
     stretches otherwise), question quiet, answer plain readable text. */
  .ask-turns {
    max-height: 320px;
    overflow-y: auto;
    padding: 0.25rem 0.9rem 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
  }
  .ask-you {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-shell) 48%, transparent);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ask-answer {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: color-mix(in srgb, var(--color-fg-shell) 90%, transparent);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ask-caret {
    display: inline-block;
    width: 0.45em;
    height: 1em;
    margin-left: 0.15em;
    vertical-align: text-bottom;
    background: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    animation: ask-blink 1s steps(2, start) infinite;
  }
  @keyframes ask-blink {
    to {
      visibility: hidden;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ask-caret {
      animation: none;
    }
  }

  .ask-note {
    margin: 0;
    padding: 0.25rem 0.9rem 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
  }

  /* The escalation footer, in the launcher's existing footer voice. */
  .ask-footer {
    display: flex;
    gap: 1rem;
    padding: 0.5rem 0.9rem 0.6rem;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);
  }
</style>
