<script lang="ts">
  /// Audio popover: output/input volume, device selection, per-app volume, DND.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "@lunaris/ui-kit/components/ui/separator/index.js";
  import { PopoverSelect } from "@lunaris/ui-kit/components/ui/popover-select";
  import {
    Volume2, VolumeX, Mic, MicOff, ChevronRight,
  } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import { FillSlider } from "@lunaris/ui-kit/components/ui/fill-slider";

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
  let appsExpanded = $state(false);

  /// Projections for the device pickers. `PopoverSelect` takes a flat
  /// `{value, label}[]`, and `value` is the currently-selected id.
  const outputOptions = $derived(
    outputs.map((o) => ({ value: o.id, label: o.name })),
  );
  const inputOptions = $derived(
    inputs.map((i) => ({ value: i.id, label: i.name })),
  );
  const currentOutputId = $derived(outputs.find((o) => o.is_default)?.id ?? "");
  const currentInputId = $derived(inputs.find((i) => i.is_default)?.id ?? "");

  interface AudioFullState {
    status: { volume: number; muted: boolean; output_type: string };
    input_status: { volume: number; muted: boolean };
    outputs: AudioDevice[];
    inputs: AudioDevice[];
    apps: AppVol[];
  }

  async function poll() {
    try {
      const r = await invoke<AudioFullState>("get_audio_full_state");
      volume = r.status.volume;
      muted = r.status.muted;
      inputVolume = r.input_status.volume;
      inputMuted = r.input_status.muted;
      outputs = r.outputs;
      inputs = r.inputs;
      apps = r.apps;
    } catch {}
  }

  // PopoverSelect owns its open-state internally and unmounts cleanly
  // with the surrounding `{#if $activePopover === "audio"}` guard, so
  // the old dropdown-open flags are no longer needed.
  $effect(() => {
    if ($activePopover === "audio") {
      poll();
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
    invoke("set_audio_output", { id }).then(() => poll()).catch(() => {});
  }
  function selectInput(id: string) {
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
  <div class="pop-panel pop-audio shell-popover" onclick={(e) => e.stopPropagation()}>
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
        <div class="vol-slider-wrap">
          <FillSlider
            value={volume}
            min={0}
            max={100}
            step={1}
            size="sm"
            ariaLabel="Output volume"
            oninput={(v) => setVolume(v)}
          />
        </div>
        <span class="vol-value">{volume}%</span>
      </div>

      <PopoverSelect
        value={currentOutputId}
        options={outputOptions}
        onchange={selectOutput}
        ariaLabel="Audio output device"
        width="100%"
      />

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
          <div class="vol-slider-wrap">
            <FillSlider
              value={inputVolume}
              min={0}
              max={100}
              step={1}
              size="sm"
              ariaLabel="Input volume"
              oninput={(v) => setInputVol(v)}
            />
          </div>
          <span class="vol-value">{inputVolume}%</span>
        </div>

        <PopoverSelect
          value={currentInputId}
          options={inputOptions}
          onchange={selectInput}
          ariaLabel="Audio input device"
          width="100%"
        />

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
            {#each apps as app (app.name)}
              <div class="app-row">
                <div class="app-icon">
                  {#if app.icon_data}
                    <img src={app.icon_data} alt="" class="app-icon-img" />
                  {:else}
                    <span class="app-icon-letter">{app.name.charAt(0).toUpperCase()}</span>
                  {/if}
                </div>
                <span class="app-name" title={app.name}>{app.name}</span>
                <div class="vol-slider-wrap app-slider-wrap">
                  <FillSlider
                    value={app.volume}
                    min={0}
                    max={100}
                    step={1}
                    size="sm"
                    ariaLabel="{app.name} volume"
                    oninput={(v) => setAppVol(app.id, v)}
                  />
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
    position: fixed; top: 40px; z-index: 100; border-radius: var(--radius-card);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg); color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: lunaris-popover-in var(--duration-medium) var(--ease-out) both;
    transform-origin: top center;
  }
  .pop-audio { right: 80px; width: 280px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  /* Entry keyframes defined in sdk/ui-kit/src/lib/motion.css. */

  .section-label { font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }

  /* Volume row */
  .vol-row { display: flex; align-items: center; gap: 8px; }
  .vol-icon-btn {
    width: var(--height-control, 28px); height: var(--height-control, 28px); display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    cursor: pointer; padding: 0; flex-shrink: 0;
    transition: all 100ms ease;
  }
  .vol-icon-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }
  .vol-value { font-size: 0.6875rem; opacity: 0.5; min-width: 30px; text-align: right; }

  /* Slider wrappers — sizing only; the bar itself comes from FillSlider. */
  .vol-slider-wrap { flex: 1; display: flex; align-items: center; }
  .app-slider-wrap { width: 100px; flex: none; }

  /*
   * Output/input device pickers use the shared PopoverSelect from
   * $lib/components/ui/popover-select. Its menu portals to
   * document.body, so styling is driven by the shell's :root theme
   * tokens (--foreground, --background) — no shell-specific overrides
   * live here.
   */

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
    width: var(--height-control-compact, 24px); height: var(--height-control-compact, 24px); display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-radius: var(--radius-chip); flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
  }
  .app-icon-img { width: 16px; height: 16px; object-fit: contain; border-radius: var(--radius-chip); }
  .app-icon-letter { font-size: 0.6875rem; font-weight: 600; color: var(--color-fg-shell); }
  .app-name { font-size: 0.6875rem; flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
