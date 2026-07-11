<script lang="ts">
  /// Error-tinted banner for ledger-health warnings (tampered ledger,
  /// behaviour directories that failed to load).
  import type { Snippet } from "svelte";
  import { ShieldAlert } from "@lucide/svelte";

  let { children }: { children?: Snippet } = $props();
</script>

<div class="banner" role="alert">
  <ShieldAlert size={16} strokeWidth={1.75} />
  <div class="banner-body">
    {@render children?.()}
  </div>
</div>

<style>
  .banner {
    display: flex;
    align-items: flex-start;
    gap: 0.5rem;
    padding: 0.625rem var(--space-row, 0.75rem);
    font-size: var(--text-sm);
    line-height: 1.4;
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
  }
  .banner :global(svg) {
    flex-shrink: 0;
    margin-top: 0.1rem;
  }
  .banner-body {
    min-width: 0;
  }
</style>
