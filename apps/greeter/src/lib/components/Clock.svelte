<script lang="ts">
  /// The greeter clock: a large time over the long date, centered at the
  /// top. Locale-aware via Intl, re-synced on the minute boundary so it
  /// never drifts (the ClockIndicator pattern from the desktop shell).
  import { onMount } from "svelte";

  const locale = typeof navigator !== "undefined" ? navigator.language || "en" : "en";
  const timeFmt = new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit" });
  const dateFmt = new Intl.DateTimeFormat(locale, {
    weekday: "long",
    day: "numeric",
    month: "long",
  });

  // A fixed clock for stable screenshots: the mock can pin the time by
  // passing `now`. Live otherwise.
  let { now = null }: { now?: Date | null } = $props();

  let time = $state("");
  let date = $state("");

  function render(d: Date) {
    time = timeFmt.format(d);
    date = dateFmt.format(d);
  }

  onMount(() => {
    if (now) {
      render(now);
      return;
    }
    render(new Date());
    const d = new Date();
    const msToMinute = (60 - d.getSeconds()) * 1000 - d.getMilliseconds();
    let interval: ReturnType<typeof setInterval>;
    const timer = setTimeout(() => {
      render(new Date());
      interval = setInterval(() => render(new Date()), 60_000);
    }, msToMinute);
    return () => {
      clearTimeout(timer);
      clearInterval(interval);
    };
  });
</script>

<div class="clock">
  <div class="time">{time}</div>
  <div class="date">{date}</div>
</div>

<style>
  .clock {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.375rem;
    text-align: center;
  }
  .time {
    font-size: calc(3.75rem * var(--greeter-scale, 1));
    font-weight: 300;
    line-height: 1;
    letter-spacing: -0.01em;
    font-variant-numeric: tabular-nums;
    color: var(--foreground);
  }
  .date {
    font-size: calc(0.875rem * var(--greeter-scale, 1));
    font-weight: 400;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
