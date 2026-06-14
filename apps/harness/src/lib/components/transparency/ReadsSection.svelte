<script lang="ts">
  /// Reads: what the AI has actually read. The audit-reader feed (LCG-R8)
  /// is not built, so this renders "not measured yet" and never a false
  /// "nothing read" (the honesty discipline that keeps the surface from
  /// lying). When the feed lands, every read file is listed here.
  import type { Capability } from "$lib/capability";
  import SectionState from "./SectionState.svelte";

  let { capability }: { capability: Capability | null } = $props();

  const off = $derived(capability !== null && !capability.enabled);
</script>

{#if off}
  <SectionState
    tag="AI is off"
    tone="off"
    message="The AI is off, so it is reading nothing."
  />
{:else}
  <SectionState
    tag="Not measured yet"
    tone="info"
    message="Arlen does not record which of your files the assistant has opened yet."
    hint="When that measurement is in place, every file it reads appears here, so a quiet pattern of reading would be plain to see."
  />
{/if}
