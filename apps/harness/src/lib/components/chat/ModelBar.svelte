<script lang="ts">
  /// The composer's top edge: which model answers. Read from the host's
  /// `ai_active`, which reads the config the engine itself follows. A fact and
  /// not a control: the provider is one of the switches only Settings may
  /// change, so choosing a model happens there.
  /// The Svelte-5 IPC caveat applies, so the value lives in a `writable` store.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";

  const active = writable<{ provider: string; model: string } | null>(null);

  onMount(async () => {
    try {
      const sel = JSON.parse(await invoke<string>("ai_active")) as {
        provider?: string;
        model?: string;
      };
      active.set(sel.provider && sel.model ? { provider: sel.provider, model: sel.model } : null);
    } catch {
      active.set(null);
    }
  });
</script>

{#if $active}
  <div class="model-bar">
    <span class="model-fact">{$active.provider} {$active.model}</span>
  </div>
{/if}

<style>
  .model-bar {
    display: flex;
    align-items: center;
    padding: 0.125rem 0.25rem;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .model-fact {
    padding: 0.25rem 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
