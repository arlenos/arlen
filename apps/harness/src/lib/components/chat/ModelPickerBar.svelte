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
      // `null` means the daemon was not reached, which is not the same as a
      // daemon that answered with no models. Neither draws a picker - there is
      // nothing to pick either way - but only the second is a measurement, and
      // the command no longer answers `"[]"` for the first. The outage itself is
      // spoken for by `CapabilityBar`, which reads the same daemon and prints
      // `h.capability.unreachable` with a retry one line below this bar.
      const json = await invoke<string | null>("ai_models_list");
      models.set(json === null ? [] : parse<ModelEntry[]>(json, []));
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
{:else if $active}
  <!-- Nothing to pick from, but the model in use is still a fact worth
       seeing (design-system.md 6.6), so the bar states it. -->
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
