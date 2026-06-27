<script lang="ts">
  /// The composer's top-edge model bar: loads the catalogue + the live
  /// selection from the daemon (`ai_models_list` / `ai_active`, both JSON
  /// strings off the Tauri bridge), renders the searchable `ModelPicker`, and
  /// commits a pick live via `ai_set_active`. The Svelte-5 IPC caveat applies
  /// (state mutated from a Tauri callback does not re-render reliably), so the
  /// reactive data lives in `writable` stores. Fails quiet: an unreachable
  /// daemon yields an empty catalogue and the picker hides itself.
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import ModelPicker, { type ModelEntry } from "./ModelPicker.svelte";

  const models = writable<ModelEntry[]>([]);
  const active = writable<{ provider: string; model: string } | null>(null);

  function parse<T>(json: string, fallback: T): T {
    try {
      return JSON.parse(json) as T;
    } catch {
      return fallback;
    }
  }

  async function loadCatalogue() {
    try {
      const list = parse<ModelEntry[]>(await invoke<string>("ai_models_list"), []);
      models.set(list);
    } catch {
      models.set([]);
    }
  }
  async function loadActive() {
    try {
      const sel = parse<{ provider?: string; model?: string }>(
        await invoke<string>("ai_active"),
        {},
      );
      active.set(sel.provider && sel.model ? { provider: sel.provider, model: sel.model } : null);
    } catch {
      active.set(null);
    }
  }

  onMount(() => {
    loadCatalogue();
    loadActive();
  });

  /// Commit a live swap. The daemon returns the new `{provider, model}` on
  /// success; on a refused swap it throws, and the selection stays put.
  async function select(provider: string, model: string) {
    try {
      const res = parse<{ provider?: string; model?: string }>(
        await invoke<string>("ai_set_active", { provider, model }),
        {},
      );
      if (res.provider && res.model) {
        active.set({ provider: res.provider, model: res.model });
      }
    } catch {
      // Refused swap (unknown provider, proxy down): keep the current model.
    }
  }
</script>

{#if $models.length > 0}
  <div class="model-bar">
    <ModelPicker models={$models} active={$active} onselect={select} />
  </div>
{/if}

<style>
  .model-bar {
    display: flex;
    align-items: center;
    padding: 0.125rem 0.25rem;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
</style>
