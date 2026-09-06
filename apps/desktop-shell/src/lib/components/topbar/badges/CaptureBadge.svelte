<script lang="ts">
  /// Top-bar capture/sharing indicator (screenshot-capture-plan.md §4): the
  /// no-silent-capture invariant made visible. Shown whenever the screen is being
  /// captured or screencast; pulsates to draw the eye; a click stops it (the
  /// operator who sees it is positioned to kill it). Mirrors RecordingBadge.
  ///
  /// Mock-vs-live: the capture-state signal (compositor/portal -> shell) is a coder
  /// seam. Under vite dev the badge shows a fixture so the surface renders; on metal
  /// without the command it stays invisible (no cry-wolf), like RecordingBadge.
  import { t } from "$lib/i18n/messages";
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { tauriAvailable } from "$lib/tauri";
  import { ScreenShare } from "lucide-svelte";
  import { shellRead } from "$lib/shellRead";
  import { shellAction } from "$lib/shellAction";
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
    // Through `shellRead` rather than a bare catch, and the difference is not
    // cosmetic on THIS badge. A failing poll keeps the last known value, which is
    // the right behaviour - blanking on one bad read would flicker, and the safe
    // direction for a privacy light is to keep claiming a share rather than drop
    // it. But the label beside it is a clock, so a reader that has been dead for
    // an hour shows a confident, growing duration for a state nobody can still
    // confirm. Keeping the light and saying nothing in the log is what made the
    // two indistinguishable; `shellRead` writes the line once per distinct
    // failure, so a session can tell them apart.
    const s = await shellRead<CaptureStatus>("capture_status", "capture");
    if (s !== null) {
      active = s.captureActive;
      app = s.capturingAppLabel ?? "an app";
      startedAt = s.startedAt ?? null;
      return;
    }
    if (!tauriAvailable) {
      active = true;
      app = "Meet";
      if (startedAt === null) startedAt = Date.now() - 47_000;
    }
  }

  function handleClick() {
    // A swallowed refusal is worse here than anywhere else in the top bar. This
    // badge exists because the operator who sees it is the one positioned to kill
    // the share; a Stop that quietly does nothing leaves them believing they did.
    // The sibling recording badge has named its failure since it was written and
    // this one never did. One clause is enough - the badge stays lit, so the
    // screen already says the share is still running.
    void shellAction("stop_capture", {}, "sh.badge.errCapture").then(() => refresh());
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
  title={$t("sh.aria.sharing", { app })}
  onclick={handleClick}
>
  {#snippet icon()}
    <ScreenShare size={12} strokeWidth={2} />
  {/snippet}
</StatusBadge>
