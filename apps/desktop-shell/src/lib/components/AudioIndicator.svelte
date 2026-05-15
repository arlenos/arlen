<script lang="ts">
  /// Audio volume indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive. Polls wpctl via Tauri
  /// (event-driven with a freshness fallback). Click toggles the
  /// popover; scroll wheel adjusts volume in 5% steps.
  ///
  /// Future MPRIS expansion will land here as the `label` slot —
  /// the Applet primitive already supports an inline label that
  /// truncates at `--topbar-applet-label-max-w`.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet } from "@lunaris/ui-kit/components/topbar";
  import { VolumeX, Volume, Volume1, Volume2, Headphones, Speaker, Monitor } from "lucide-svelte";

  interface AudioStatus {
    volume: number;
    muted: boolean;
    output_type: string;
  }

  let status = $state<AudioStatus | null>(null);

  async function poll() {
    try {
      status = await invoke<AudioStatus>("get_audio_status");
    } catch {
      status = null;
    }
  }

  poll();

  const POLL_STALE_MS = 90_000;
  let lastEventAt = Date.now();

  onMount(() => {
    const unlisten = listen("audio-changed", () => {
      lastEventAt = Date.now();
      poll();
    });
    const fallback = setInterval(() => {
      if (Date.now() - lastEventAt < POLL_STALE_MS) return;
      poll();
    }, 30_000);
    return () => {
      unlisten.then((fn) => fn());
      clearInterval(fallback);
    };
  });

  const outputType = $derived(status?.output_type ?? "speakers");

  const Icon = $derived(
    !status || status.muted || status.volume === 0
      ? VolumeX
      : outputType === "bluetooth_headphones"
        ? Headphones
        : outputType === "bluetooth_speaker"
          ? Speaker
          : outputType === "hdmi"
            ? Monitor
            : status.volume <= 33
              ? Volume
              : status.volume <= 66
                ? Volume1
                : Volume2,
  );

  const tooltip = $derived(
    !status
      ? "Audio"
      : status.muted
        ? "Volume: Muted"
        : `Volume: ${status.volume}%`,
  );

  const isOpen = $derived($activePopover === "audio");

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    if (!status) return;
    const delta = e.deltaY < 0 ? 5 : -5;
    const newVol = Math.max(0, Math.min(100, status.volume + delta));
    invoke("set_audio_volume", { volume: newVol })
      .then(() => poll())
      .catch(() => {});
  }
</script>

{#if status}
  <Applet
    appletId="audio"
    {tooltip}
    popoverOpen={isOpen}
    state={status.muted ? "off" : "on"}
    onclick={() => togglePopover("audio")}
    onmouseenter={() => hoverPopover("audio")}
    onWheel={handleWheel}
  >
    {#snippet icon()}
      <Icon size={14} strokeWidth={1.5} />
    {/snippet}
  </Applet>
{/if}
