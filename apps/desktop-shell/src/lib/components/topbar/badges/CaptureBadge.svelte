<script lang="ts">
  /// Top-bar capture/sharing indicator (screenshot-capture-plan.md §4): the
  /// no-silent-capture invariant made visible. Shown whenever the screen is being
  /// captured or screencast; pulsates to draw the eye; a click stops it (the
  /// operator who sees it is positioned to kill it). Mirrors RecordingBadge.
  ///
  /// Mock-vs-live: the capture-state signal (compositor/portal -> shell) is a coder
  /// seam. Under vite dev the badge shows a fixture so the surface renders; on metal
  /// without the command it stays invisible (no cry-wolf), like RecordingBadge.
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { ScreenShare } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  interface CaptureStatus {
    captureActive: boolean;
    capturingAppLabel?: string;
    startedAt?: number;
  }

  let active = $state(false);
  let app = $state("an app");
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
      const s = await invoke<CaptureStatus>("capture_status");
      active = s.captureActive;
      app = s.capturingAppLabel ?? "an app";
      startedAt = s.startedAt ?? null;
    } catch {
      if (import.meta.env.DEV) {
        active = true;
        app = "Meet";
        if (startedAt === null) startedAt = Date.now() - 47_000;
      }
    }
  }

  function handleClick() {
    invoke("stop_capture")
      .then(() => refresh())
      .catch(() => {});
  }

  const elapsed = $derived(
    active && startedAt ? Math.max(0, Math.floor((now - startedAt) / 1000)) : 0,
  );
  const label = $derived(
    elapsed === 0
      ? "LIVE"
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
  title={`Your screen is being shared with ${app}. Click to stop.`}
  onclick={handleClick}
>
  {#snippet icon()}
    <ScreenShare size={12} strokeWidth={2} />
  {/snippet}
</StatusBadge>
