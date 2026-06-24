<script lang="ts">
  /// The origin marker IS the prompt char (terminal.md §4.7): a
  /// command the user typed gets the normal prompt character, silent
  /// by design; a command the agent drafted gets `✦` — origin where
  /// the eye already is, and the only AI presence the resting
  /// terminal carries (corrected 11 June: contextual, never an
  /// always-on ornament). Origin is backend-stamped — this component
  /// only renders what the contract says, it never infers.
  import type { Origin } from "$lib/contract";

  let { origin }: { origin: Origin } = $props();
</script>

{#if origin === "agent"}
  <span class="om om-agent" role="img" aria-label="Drafted by the assistant">✦</span>
{/if}

<style>
  /* Provenance is quiet-chrome: a command the user ran carries no marker at
     all (the block already says everything), so only an agent-drafted command
     shows the ✦ — origin where the eye already is. */
  .om-agent {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: var(--color-accent, var(--primary));
  }
</style>
