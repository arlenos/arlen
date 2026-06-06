<script lang="ts">
  /// Top-bar Recording badge.
  ///
  /// Visible only while a screen recording is in progress. Renders
  /// a live elapsed-time counter beside the icon and pulsates to
  /// draw the eye.
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { Circle } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  interface ToggleStatus {
    recordingActive: boolean;
    recordingStartedAt?: number;
  }

  let active = $state(false);
  let startedAt = $state<number | null>(null);
  let now = $state(Date.now());
  let pollInterval: ReturnType<typeof setInterval> | null = null;
  let tickInterval: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    refresh();
    pollInterval = setInterval(refresh, 4_000);
    tickInterval = setInterval(() => {
      now = Date.now();
    }, 1_000);
    return () => {
      if (pollInterval) clearInterval(pollInterval);
      if (tickInterval) clearInterval(tickInterval);
    };
  });

  async function refresh() {
    try {
      const s = await invoke<ToggleStatus>("get_toggle_status");
      active = s.recordingActive;
      startedAt = s.recordingStartedAt ?? null;
    } catch {}
  }

  function handleClick() {
    invoke("quick_action_run", { id: "qa.toggle_recording" })
      .then(() => refresh())
      .catch(() => {});
  }

  const elapsed = $derived(
    active && startedAt ? Math.max(0, Math.floor((now - startedAt) / 1000)) : 0,
  );
  const label = $derived(
    elapsed === 0
      ? "REC"
      : `${Math.floor(elapsed / 60)
          .toString()
          .padStart(2, "0")}:${(elapsed % 60).toString().padStart(2, "0")}`,
  );
</script>

<StatusBadge
  visible={active}
  active={active}
  pulsate
  label={label}
  title="Recording in progress. Click to stop."
  onclick={handleClick}
>
  {#snippet icon()}
    <Circle size={10} strokeWidth={2.5} fill="currentColor" />
  {/snippet}
</StatusBadge>
