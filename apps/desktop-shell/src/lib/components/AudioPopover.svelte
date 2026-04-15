<script lang="ts">
  /// Audio popover: output/input volume, device selection, per-app volume, DND.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import {
    Volume2, VolumeX, Mic, MicOff, ChevronDown, ChevronRight, Check,
  } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";

  interface AudioDevice { id: string; name: string; is_default: boolean; }
  interface AppVol { id: number; name: string; volume: number; icon_data: string | null; }

  let volume = $state(75);
  let muted = $state(false);
  let inputVolume = $state(50);
  let inputMuted = $state(false);
  let dndEnabled = $state(false);
  let outputs = $state<AudioDevice[]>([]);
  let inputs = $state<AudioDevice[]>([]);
  let apps = $state<AppVol[]>([]);
  let outputDropdownOpen = $state(false);
  let inputDropdownOpen = $state(false);
  let appsExpanded = $state(false);

  async function poll() {
    try {
      const r = await invoke<{ volume: number; muted: boolean }>("get_audio_status");
      volume = r.volume; muted = r.muted;
    } catch {}
    try {
      const r = await invoke<{ volume: number; muted: boolean }>("get_input_volume");
      inputVolume = r.volume; inputMuted = r.muted;
    } catch {}
    try { outputs = await invoke<AudioDevice[]>("get_audio_outputs"); } catch {}
    try { inputs = await invoke<AudioDevice[]>("get_audio_inputs"); } catch {}
  }

  async function loadApps() {
    try { apps = await invoke<AppVol[]>("get_app_volumes"); } catch {}
  }

  $effect(() => {
    if ($activePopover === "audio") {
      poll();
      loadApps();
    } else {
      outputDropdownOpen = false;
      inputDropdownOpen = false;
    }
  });

  function setVolume(val: number) {
    volume = val;
    invoke("set_audio_volume", { volume: val }).catch(() => {});
  }
  function toggleMute() {
    invoke("toggle_audio_mute").then(() => poll()).catch(() => {});
  }
  function setInputVol(val: number) {
    inputVolume = val;
    invoke("set_input_volume", { volume: val }).catch(() => {});
  }
  function toggleInputMute() {
    invoke("toggle_input_mute").then(() => poll()).catch(() => {});
  }
  function selectOutput(id: string) {
    outputDropdownOpen = false;
    invoke("set_audio_output", { id }).then(() => poll()).catch(() => {});
  }
  function selectInput(id: string) {
    inputDropdownOpen = false;
    invoke("set_audio_input", { id }).then(() => poll()).catch(() => {});
  }
  function setAppVol(id: number, val: number) {
    const app = apps.find(a => a.id === id);
    if (app) app.volume = val;
    invoke("set_app_volume", { id, volume: val }).catch(() => {});
  }
  function toggleDnd() {
    dndEnabled = !dndEnabled;
    invoke("set_dnd_enabled", { enabled: dndEnabled }).catch(() => {});
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
    <PopoverHeader icon={Volume2} title="Sound" toggled={!dndEnabled} onToggle={toggleDnd} />
    <div class="pop-body">

      <!-- Output Section -->
      <div class="section-label">Output</div>
      <div class="vol-row">
        <button class="vol-icon-btn" onclick={(e) => { e.stopPropagation(); toggleMute(); }}
          aria-label={muted ? "Unmute" : "Mute"}>
          {#if muted}
            <VolumeX size={16} strokeWidth={1.5} />
          {:else}
            <Volume2 size={16} strokeWidth={1.5} />
          {/if}
        </button>
        <div class="vol-slider" style="--value: {volume}%">
          <div class="vol-slider-track"></div>
          <div class="vol-slider-fill"></div>
          <div class="vol-slider-thumb"></div>
          <input type="range" min="0" max="100" value={volume}
            oninput={(e) => setVolume(parseInt(e.currentTarget.value))} />
        </div>
        <span class="vol-value">{volume}%</span>
      </div>

      <div class="cs-wrap">
        <button class="cs-trigger" onclick={(e) => { e.stopPropagation(); outputDropdownOpen = !outputDropdownOpen; inputDropdownOpen = false; }}>
          <span class="cs-value">{outputs.find((o) => o.is_default)?.name ?? "Default"}</span>
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

      <Separator class="opacity-10" />

      <!-- Input Section -->
      {#if inputs.length > 0}
        <div class="section-label">Input</div>
        <div class="vol-row">
          <button class="vol-icon-btn" onclick={(e) => { e.stopPropagation(); toggleInputMute(); }}
            aria-label={inputMuted ? "Unmute mic" : "Mute mic"}>
            {#if inputMuted}
              <MicOff size={16} strokeWidth={1.5} />
            {:else}
              <Mic size={16} strokeWidth={1.5} />
            {/if}
          </button>
          <div class="vol-slider" style="--value: {inputVolume}%">
            <div class="vol-slider-track"></div>
            <div class="vol-slider-fill"></div>
            <div class="vol-slider-thumb"></div>
            <input type="range" min="0" max="100" value={inputVolume}
              oninput={(e) => setInputVol(parseInt(e.currentTarget.value))} />
          </div>
          <span class="vol-value">{inputVolume}%</span>
        </div>

        <div class="cs-wrap">
          <button class="cs-trigger" onclick={(e) => { e.stopPropagation(); inputDropdownOpen = !inputDropdownOpen; outputDropdownOpen = false; }}>
            <span class="cs-value">{inputs.find((i) => i.is_default)?.name ?? "Default"}</span>
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

        <Separator class="opacity-10" />
      {/if}

      <!-- Per-App Volume (Collapsible) -->
      {#if apps.length > 0}
        <button class="apps-header" onclick={(e) => { e.stopPropagation(); appsExpanded = !appsExpanded; }}>
          <ChevronRight size={12} strokeWidth={2} class={appsExpanded ? "apps-chevron-open" : ""} />
          <span>Apps ({apps.length})</span>
        </button>
        {#if appsExpanded}
          <div class="apps-list">
            {#each apps as app}
              <div class="app-row">
                <div class="app-icon">
                  {#if app.icon_data}
                    <img src={app.icon_data} alt="" class="app-icon-img" />
                  {:else}
                    <span class="app-icon-letter">{app.name.charAt(0).toUpperCase()}</span>
                  {/if}
                </div>
                <span class="app-name" title={app.name}>{app.name}</span>
                <div class="vol-slider app-slider" style="--value: {app.volume}%">
                  <div class="vol-slider-track"></div>
                  <div class="vol-slider-fill"></div>
                  <div class="vol-slider-thumb"></div>
                  <input type="range" min="0" max="100" value={app.volume}
                    oninput={(e) => setAppVol(app.id, parseInt(e.currentTarget.value))} />
                </div>
                <span class="vol-value">{app.volume}%</span>
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }
  .pop-panel {
    position: fixed; top: 40px; z-index: 100; border-radius: var(--radius-lg);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg); color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: pop-open 100ms ease-out both;
  }
  .pop-audio { right: 80px; width: 280px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  .section-label { font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }

  /* Volume row */
  .vol-row { display: flex; align-items: center; gap: 8px; }
  .vol-icon-btn {
    width: 28px; height: 28px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: var(--radius-sm);
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    cursor: pointer; padding: 0; flex-shrink: 0;
    transition: all 100ms ease;
  }
  .vol-icon-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }
  .vol-value { font-size: 0.6875rem; opacity: 0.5; min-width: 30px; text-align: right; }

  /* Slider */
  .vol-slider { position: relative; flex: 1; height: 20px; display: flex; align-items: center; }
  .vol-slider-track { position: absolute; left: 0; right: 0; height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: var(--radius-sm); }
  .vol-slider-fill { position: absolute; left: 0; width: var(--value); height: 4px; background: var(--color-accent); border-radius: var(--radius-sm); }
  .vol-slider-thumb { position: absolute; left: var(--value); width: 14px; height: 14px; background: var(--color-fg-shell); border-radius: var(--radius-md); transform: translateX(-50%); box-shadow: var(--shadow-sm); pointer-events: none; }
  .vol-slider input[type="range"] { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; margin: 0; appearance: none; -webkit-appearance: none; }
  .app-slider { width: 100px; flex: none; }
  .app-slider .vol-slider-thumb { width: 12px; height: 12px; }

  /* Custom Select */
  .cs-wrap { position: relative; }
  .cs-trigger {
    width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding: 5px 8px; border-radius: var(--radius-md);
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
    border-radius: var(--radius-md); padding: 4px;
    box-shadow: var(--shadow-md);
    z-index: 10; max-height: 160px; overflow-y: auto;
  }
  .cs-item {
    width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 6px 8px; background: transparent; border: none; border-radius: var(--radius-sm);
    color: var(--color-fg-shell); font-size: 0.6875rem; cursor: pointer; text-align: left;
    transition: background-color 0.1s ease;
  }
  .cs-item:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .cs-item.selected { background: color-mix(in srgb, var(--color-accent) 15%, transparent); }

  /* Apps section */
  .apps-header {
    display: flex; align-items: center; gap: 6px;
    padding: 4px 0; background: transparent; border: none;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    font-size: 0.75rem; font-weight: 500; cursor: pointer; width: 100%; text-align: left;
    transition: color 0.1s ease;
  }
  .apps-header:hover { color: var(--color-fg-shell); }
  :global(.apps-chevron-open) { transform: rotate(90deg); }
  .apps-list { display: flex; flex-direction: column; gap: 6px; }
  .app-row { display: flex; align-items: center; gap: 6px; }
  .app-icon {
    width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-radius: var(--radius-sm); flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
  }
  .app-icon-img { width: 16px; height: 16px; object-fit: contain; border-radius: var(--radius-sm); }
  .app-icon-letter { font-size: 0.6875rem; font-weight: 600; color: var(--color-fg-shell); }
  .app-name { font-size: 0.6875rem; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
