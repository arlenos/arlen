<script lang="ts">
  /// Wrapper that lazy-loads the current config value for a settings
  /// result's inline action. Renders the appropriate inline control
  /// (toggle or pills) once the value is known.

  import { onMount } from "svelte";
  import {
    getSettingValue,
    setSettingValue,
    searchSettings,
    type InlineAction,
  } from "$lib/stores/settingsSearch.js";
  import WaypointerInlineToggle from "./WaypointerInlineToggle.svelte";
  import WaypointerInlinePills from "./WaypointerInlinePills.svelte";

  let {
    action,
    query,
  }: {
    action: InlineAction;
    query: string;
  } = $props();

  let value = $state<unknown>(null);
  let loaded = $state(false);

  onMount(() => {
    getSettingValue(action.configFile, action.configKey).then((v) => {
      value = v;
      loaded = true;
    });
  });

  async function setValue(v: unknown) {
    value = v; // optimistic
    await setSettingValue(action.configFile, action.configKey, v);
    searchSettings(query);
  }
</script>

{#if loaded}
  {#if action.actionType === "toggle"}
    <WaypointerInlineToggle
      checked={value === true}
      onchange={(v) => setValue(v)}
    />
  {:else if action.actionType === "select" && action.options}
    <WaypointerInlinePills
      value={String(value ?? "")}
      options={action.options}
      onchange={(v) => setValue(v)}
    />
  {/if}
{/if}
