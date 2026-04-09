<script lang="ts">
  /// Audio volume indicator for the top bar.
  ///
  /// Polls wpctl via Tauri every 2 seconds. Click toggles mute,
  /// scroll wheel adjusts volume in 5% steps.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { VolumeX, Volume, Volume1, Volume2, Headphones, Speaker, Monitor } from "lucide-svelte";

  interface AudioStatus {
    volume: number;
    muted: boolean;
    output_type: string;
  }

  let status = $state<AudioStatus | null>(null);

  const tooltipClass =
    "rounded-md border px-2 py-0.5 text-xs shadow-md select-none"
    + " bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  async function poll() {
    try {
      status = await invoke<AudioStatus>("get_audio_status");
    } catch {
      status = null;
    }
  }

  poll();
  onMount(() => {
    const unlisten = listen("audio-changed", () => poll());
    const fallback = setInterval(poll, 30_000);
    return () => {
      unlisten.then((fn) => fn());
      clearInterval(fallback);
    };
  });

  const outputType = $derived(status?.output_type ?? "speakers");

  const Icon = $derived(
    !status || status.muted || status.volume === 0 ? VolumeX :
    outputType === "bluetooth_headphones" ? Headphones :
    outputType === "bluetooth_speaker" ? Speaker :
    outputType === "hdmi" ? Monitor :
    status.volume <= 33 ? Volume :
    status.volume <= 66 ? Volume1 :
    Volume2
  );

  const label = $derived(
    !status ? "Audio" :
    status.muted ? "Volume: Muted" :
    `Volume: ${status.volume}%`
  );

  function handleClick() {
    togglePopover("audio");
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    if (!status) return;
    const delta = e.deltaY < 0 ? 5 : -5;
    const newVol = Math.max(0, Math.min(100, status.volume + delta));
    invoke("set_audio_volume", { volume: newVol }).then(() => poll()).catch(() => {});
  }

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    // TODO: Open Sound Settings.
  }
</script>

{#if status}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <button
          class="audio-btn"
          aria-label={label}
          {...props}
          onclick={handleClick}
          onwheel={handleWheel}
          oncontextmenu={handleContextMenu}
        >
          <Icon size={14} strokeWidth={1.5} />
        </button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Portal>
      <Tooltip.Content side="bottom" class={tooltipClass}>{label}</Tooltip.Content>
    </Tooltip.Portal>
  </Tooltip.Root>
{/if}

<style>
  .audio-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    min-height: 24px;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: 4px;
    cursor: pointer;
    color: var(--foreground);
    transition: background-color var(--duration-fast, 150ms) ease;
  }

  .audio-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
