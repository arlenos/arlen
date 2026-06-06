<script lang="ts">
  /// Clock indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive in a label-only
  /// configuration (no icon). Renders a dim weekday + bright time
  /// on the same row via the Applet's `labelSnippet` escape-hatch.
  /// Updates every minute, synced to the minute boundary to avoid
  /// drift.

  import { Applet } from "@arlen/ui-kit/components/topbar";

  let time = $state("");
  let weekday = $state("");

  const locale = navigator.language || "en";
  const timeFormatter = new Intl.DateTimeFormat(locale, {
    hour: "2-digit",
    minute: "2-digit",
  });
  const weekdayFormatter = new Intl.DateTimeFormat(locale, {
    weekday: "short",
  });

  function update() {
    const now = new Date();
    time = timeFormatter.format(now);
    weekday = weekdayFormatter.format(now);
  }

  update();

  let timer: ReturnType<typeof setTimeout> | null = null;
  let interval: ReturnType<typeof setInterval> | null = null;

  $effect(() => {
    const now = new Date();
    const msUntilNextMinute =
      (60 - now.getSeconds()) * 1000 - now.getMilliseconds();
    timer = setTimeout(() => {
      update();
      interval = setInterval(update, 60_000);
    }, msUntilNextMinute);
    return () => {
      if (timer) clearTimeout(timer);
      if (interval) clearInterval(interval);
    };
  });

  const tooltip = $derived(
    new Intl.DateTimeFormat(locale, {
      weekday: "long",
      day: "numeric",
      month: "long",
      year: "numeric",
    }).format(new Date()),
  );
</script>

<Applet appletId="clock" {tooltip} ariaLabel={`Clock: ${weekday} ${time}`}>
  {#snippet labelSnippet()}
    <span class="clock-weekday">{weekday}</span>
    <span class="clock-time">{time}</span>
  {/snippet}
</Applet>

<style>
  .clock-weekday {
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    line-height: 1;
    margin-right: 4px;
  }
  .clock-time {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--color-fg-shell);
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
</style>
