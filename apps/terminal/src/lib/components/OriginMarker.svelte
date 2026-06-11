<script lang="ts">
  /// The origin marker IS the prompt char (terminal.md §4.7): a
  /// command the user typed gets the normal prompt character, silent
  /// by design; a command the agent drafted gets `✦`. Origin is
  /// backend-stamped — this component only renders what the contract
  /// says, it never infers.
  import type { Origin } from "$lib/contract";

  let { origin }: { origin: Origin } = $props();
</script>

{#if origin === "agent"}
  <span
    class="om om-agent"
    role="img"
    aria-label="Drafted by the assistant"
  >✦</span>
{:else}
  <span class="om" aria-hidden="true">❯</span>
{/if}

<style>
  .om {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .om-agent {
    color: var(--color-accent, var(--primary));
  }
</style>
