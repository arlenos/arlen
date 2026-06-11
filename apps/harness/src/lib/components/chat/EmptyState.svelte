<script lang="ts">
  /// The no-messages surface, one designed variant per capability state. No
  /// decorative icon; the words and the starters carry it.
  import { Button } from "@arlen/ui-kit/components/ui/button";

  let {
    variant,
    onstarter,
    onretry,
  }: {
    variant: "ready" | "off" | "unreachable";
    /// Fills the composer with a starter prompt (editable before sending).
    onstarter: (text: string) => void;
    onretry: () => void;
  } = $props();

  // Starter prompts grounded in what the assistant can actually answer.
  const STARTERS = [
    "What did I work on yesterday?",
    "Which files belong to my current project?",
    "Where did my newest downloads go?",
  ];
</script>

<div class="empty">
  {#if variant === "ready"}
    <p class="title">Ask the assistant</p>
    <p class="sub">
      It knows your files and what you have worked on. Ask in your own words.
      It does not remember earlier questions yet.
    </p>
    <div class="starters">
      {#each STARTERS as s (s)}
        <button type="button" class="starter" onclick={() => onstarter(s)}>{s}</button>
      {/each}
    </div>
  {:else if variant === "off"}
    <p class="title">The AI is off</p>
    <p class="sub">Nothing runs while it is off. Turn it on in Settings to ask questions.</p>
  {:else}
    <p class="title">Can't reach the assistant</p>
    <p class="sub">It did not answer. Try again in a moment.</p>
    <Button variant="outline" size="sm" class="mt-2" onclick={onretry}>Try again</Button>
  {/if}
</div>

<style>
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    max-width: 28rem;
    margin-inline: auto;
    text-align: center;
    gap: 0.5rem;
  }
  .title {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .sub {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .starters {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 0.5rem;
    margin-top: 1rem;
  }
  .starter {
    display: inline-flex;
    align-items: center;
    min-height: var(--height-control, 28px);
    padding: 0 0.75rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-button);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font-size: 0.8125rem;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .starter:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    color: var(--foreground);
  }
</style>
