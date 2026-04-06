<script lang="ts">
  /// Audio popover: volume, output/input selection, DND.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Switch } from "$lib/components/ui/switch/index.js";
  import { Volume2, ChevronDown, Check } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";

  interface AudioDevice { id: string; name: string; is_default: boolean; }

  let volume = $state(75);
  let dndEnabled = $state(false);
  let outputs = $state<AudioDevice[]>([]);
  let inputs = $state<AudioDevice[]>([]);
  let outputDropdownOpen = $state(false);
  let inputDropdownOpen = $state(false);

  async function poll() {
    try {
      const r = await invoke<{ volume: number; muted: boolean }>("get_audio_status");
      volume = r.volume;
    } catch {}
    try { outputs = await invoke<AudioDevice[]>("get_audio_outputs"); } catch {}
    try { inputs = await invoke<AudioDevice[]>("get_audio_inputs"); } catch {}
  }

  $effect(() => {
    if ($activePopover === "audio") {
      poll();
    } else {
      outputDropdownOpen = false;
      inputDropdownOpen = false;
    }
  });

  function onSliderInput(e: Event) {
    const val = parseInt((e.target as HTMLInputElement).value);
    volume = val;
    invoke("set_audio_volume", { volume: val }).catch(() => {});
  }

  function selectOutput(id: string) {
    outputDropdownOpen = false;
    invoke("set_audio_output", { id }).then(() => poll()).catch(() => {});
  }

  function selectInput(id: string) {
    inputDropdownOpen = false;
    invoke("set_audio_input", { id }).then(() => poll()).catch(() => {});
  }
</script>

{#if $activePopover === "audio"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-audio shell-popover" onclick={(e) => {
    e.stopPropagation();
    outputDropdownOpen = false;
    inputDropdownOpen = false;
  }}>
    <PopoverHeader icon={Volume2} title="Sound" />
    <div class="pop-body">

      <!-- Volume -->
      <div class="pop-row">
        <Volume2 size={16} strokeWidth={1.5} class="pop-icon" />
        <div class="pop-slider" style="--value: {volume}%">
          <div class="pop-slider-track"></div>
          <div class="pop-slider-fill"></div>
          <div class="pop-slider-thumb"></div>
          <input type="range" min="0" max="100" value={volume} oninput={onSliderInput} />
        </div>
        <span class="pop-value">{volume}%</span>
      </div>

      <!-- Output -->
      {#if outputs.length > 1}
        <Separator class="opacity-10" />
        <div class="pop-row">
          <span class="pop-label">Output</span>
          <div class="cs-wrap">
            <button class="cs-trigger" onclick={(e) => { e.stopPropagation(); outputDropdownOpen = !outputDropdownOpen; inputDropdownOpen = false; }}>
              <span class="cs-value">{outputs.find((o) => o.is_default)?.name ?? "Select"}</span>
              <ChevronDown size={12} strokeWidth={2} />
            </button>
            {#if outputDropdownOpen}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="cs-dropdown" onclick={(e) => e.stopPropagation()}>
                {#each outputs as out}
                  <button class="cs-item" class:selected={out.is_default} onclick={(e) => { e.stopPropagation(); selectOutput(out.id); }}>
                    <span>{out.name}</span>
                    {#if out.is_default}<Check size={12} strokeWidth={2} />{/if}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <!-- Input -->
      {#if inputs.length > 0}
        <Separator class="opacity-10" />
        <div class="pop-row">
          <span class="pop-label">Input</span>
          <div class="cs-wrap">
            <button class="cs-trigger" onclick={(e) => { e.stopPropagation(); inputDropdownOpen = !inputDropdownOpen; outputDropdownOpen = false; }}>
              <span class="cs-value">{inputs.find((i) => i.is_default)?.name ?? "Select"}</span>
              <ChevronDown size={12} strokeWidth={2} />
            </button>
            {#if inputDropdownOpen}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="cs-dropdown" onclick={(e) => e.stopPropagation()}>
                {#each inputs as inp}
                  <button class="cs-item" class:selected={inp.is_default} onclick={(e) => { e.stopPropagation(); selectInput(inp.id); }}>
                    <span>{inp.name}</span>
                    {#if inp.is_default}<Check size={12} strokeWidth={2} />{/if}
                  </button>
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/if}

      <!-- DND -->
      <Separator class="opacity-10" />
      <div class="pop-row">
        <span class="pop-label">Do Not Disturb</span>
        <Switch bind:checked={dndEnabled} />
      </div>
    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }
  .pop-panel {
    position: fixed; top: 40px; z-index: 100; border-radius: 10px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: pop-open 100ms ease-out both;
  }
  .pop-audio { right: 80px; width: 280px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  .pop-row { display: flex; align-items: center; gap: 10px; min-height: 28px; }
  :global(.pop-icon) { opacity: 0.5; flex-shrink: 0; }
  .pop-label { font-size: 0.75rem; flex: 1; }
  .pop-value { font-size: 0.6875rem; opacity: 0.5; min-width: 32px; text-align: right; }

  /* ── Custom Select ── */
  .cs-wrap { position: relative; flex: 1; max-width: 160px; }
  .cs-trigger {
    width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding: 5px 8px; border-radius: 6px;
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: var(--color-fg-shell); font-size: 0.6875rem; cursor: pointer; text-align: left;
    transition: border-color 0.1s ease;
  }
  .cs-trigger:hover { border-color: color-mix(in srgb, var(--color-fg-shell) 25%, transparent); }
  .cs-value { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .cs-dropdown {
    position: absolute; top: 100%; left: 0; right: 0; margin-top: 4px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border-radius: 6px; padding: 4px;
    box-shadow: var(--shadow-md);
    z-index: 10; max-height: 160px; overflow-y: auto;
  }
  .cs-item {
    width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 6px 8px; background: transparent; border: none; border-radius: 4px;
    color: var(--color-fg-shell); font-size: 0.6875rem; cursor: pointer; text-align: left;
    transition: background-color 0.1s ease;
  }
  .cs-item:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .cs-item.selected { background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent); }

  /* ── Slider ── */
  .pop-slider { position: relative; flex: 1; height: 20px; display: flex; align-items: center; }
  .pop-slider-track { position: absolute; left: 0; right: 0; height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: 2px; }
  .pop-slider-fill { position: absolute; left: 0; width: var(--value); height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); border-radius: 2px; }
  .pop-slider-thumb { position: absolute; left: var(--value); width: 14px; height: 14px; background: var(--color-fg-shell); border-radius: 50%; transform: translateX(-50%); box-shadow: var(--shadow-sm); pointer-events: none; }
  .pop-slider input[type="range"] { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; margin: 0; appearance: none; -webkit-appearance: none; }
</style>
