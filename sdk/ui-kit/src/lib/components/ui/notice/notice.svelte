<script lang="ts">
  /// One inline notice line: an icon, one finished sentence, a tone border.
  /// The three tones are a statement about trust, not decoration - `error`
  /// says the content below cannot be believed, `caution` says it is doing
  /// something worth your eyes, `neutral` states a fact once. The sentence
  /// arrives finished from the app's catalogue; the kit only shapes it.
  ///
  /// For a full-width surface message use a page-level pattern instead; this
  /// is the quiet in-content strip (a message's format refusal, a mock
  /// banner, a service-down line).
  import { Info, OctagonAlert, TriangleAlert } from "@lucide/svelte";

  let {
    tone = "neutral",
    text,
    id,
    class: className,
  }: {
    tone?: "error" | "caution" | "neutral";
    text: string;
    /// Optional anchor id.
    id?: string;
    class?: string;
  } = $props();
</script>

<p class="notice {tone} {className ?? ''}" {id} role={tone === "error" ? "alert" : "status"}>
  {#if tone === "error"}
    <OctagonAlert size={14} strokeWidth={2} aria-hidden="true" />
  {:else if tone === "caution"}
    <TriangleAlert size={14} strokeWidth={2} aria-hidden="true" />
  {:else}
    <Info size={14} strokeWidth={2} aria-hidden="true" />
  {/if}
  <span>{text}</span>
</p>

<style>
  .notice {
    display: flex;
    gap: 0.55rem;
    align-items: flex-start;
    margin: 0;
    padding: 0.5rem 0.7rem;
    border-inline-start: 2px solid transparent;
    border-radius: var(--radius-input);
    font-size: var(--text-xs);
    line-height: 1.5;
  }
  .notice :global(svg) {
    flex-shrink: 0;
    margin-top: 0.15rem;
  }
  .error {
    border-color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 8%, transparent);
    color: var(--color-fg-primary);
  }
  .caution {
    border-color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning) 7%, transparent);
    color: var(--color-fg-primary);
  }
  .neutral {
    border-color: color-mix(in srgb, var(--color-fg-primary) 25%, transparent);
    background: color-mix(in srgb, var(--color-fg-primary) 3%, transparent);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
</style>
