<script lang="ts">
  /// Clock indicator for the top bar.
  ///
  /// Displays the current time using the system locale (24h or 12h).
  /// Optionally shows a short weekday prefix. Updates every minute,
  /// synced to the minute boundary to avoid drift.

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

  // Initial render.
  update();

  // Sync to the next minute boundary, then tick every 60s.
  let timer: ReturnType<typeof setTimeout> | null = null;
  let interval: ReturnType<typeof setInterval> | null = null;

  $effect(() => {
    const now = new Date();
    const msUntilNextMinute = (60 - now.getSeconds()) * 1000 - now.getMilliseconds();

    timer = setTimeout(() => {
      update();
      interval = setInterval(update, 60_000);
    }, msUntilNextMinute);

    return () => {
      if (timer) clearTimeout(timer);
      if (interval) clearInterval(interval);
    };
  });

  function handleClick() {
    // TODO: Open calendar popover (Phase 4).
  }

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    // TODO: Open "Date and Time Settings" via settings:// deep link.
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<button
  class="clock-indicator"
  onclick={handleClick}
  oncontextmenu={handleContextMenu}
  aria-label="Clock"
>
  <span class="clock-weekday">{weekday}</span>
  <span class="clock-time">{time}</span>
</button>

<style>
  .clock-indicator {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    height: 28px;
    min-width: 24px;
    min-height: 24px;
    background: transparent;
    border: none;
    border-radius: 4px;
    cursor: pointer;
    transition: background-color var(--duration-fast, 150ms) ease;
  }

  .clock-indicator:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .clock-indicator:focus-visible {
    outline: none;
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 50%, transparent);
  }

  .clock-weekday {
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    line-height: 1;
  }

  .clock-time {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
    line-height: 1;
  }
</style>
