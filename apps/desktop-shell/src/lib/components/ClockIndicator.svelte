<script lang="ts">
  /// Clock indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive in a label-only
  /// configuration (no icon). Renders a dim weekday + bright time
  /// on the same row via the Applet's `labelSnippet` escape-hatch.
  /// Updates every minute, synced to the minute boundary to avoid
  /// drift.

  import { t } from "$lib/i18n/messages";

  import { Applet } from "@arlen/ui-kit/components/topbar";
  import { locale } from "@arlen/ui-kit/i18n";

  let time = $state("");
  let weekday = $state("");

  /// Formatted in the language the user chose, not `navigator.language`.
  ///
  /// The webview reports the system locale, which is where this got "Thu" beside
  /// a German shell. One choice drives both today; a separate regional-formats
  /// setting (pick German, keep English dates) is a real thing other desktops
  /// offer and would be its own row, not a reason to ignore the one we have.
  const timeFormatter = $derived(
    new Intl.DateTimeFormat($locale, { hour: "2-digit", minute: "2-digit" }),
  );
  const weekdayFormatter = $derived(
    new Intl.DateTimeFormat($locale, { weekday: "short" }),
  );

  function update() {
    const now = new Date();
    time = timeFormatter.format(now);
    weekday = weekdayFormatter.format(now);
  }

  update();

  // Re-format when the LANGUAGE changes, not only when the minute does.
  //
  // The formatters follow `$locale`, but the rendered strings were written by an
  // imperative `update()` that ran at init and then once a minute - so between
  // `initArlenLocale` resolving and the next minute boundary the bar kept
  // whatever it had formatted first. Two boots of the same image with the same
  // preloaded `[locale] ui = "de"` on 23 August: one showed `So 15:09`, the other
  // `Sun 03:21 PM`, and which one you got depended on where the shot landed in
  // that window. A clock that is English for up to a minute after every start is
  // not a clock somebody can trust to be in their language.
  $effect(() => {
    // Named so the dependency is the formatters, which is what carries the
    // locale; reading them is the whole point of the line.
    timeFormatter;
    weekdayFormatter;
    update();
  });

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

  const dateFormatter = $derived(
    new Intl.DateTimeFormat($locale, {
      weekday: "long",
      day: "numeric",
      month: "long",
      year: "numeric",
    }),
  );
  // `time` is the reactive dependency: the full-date tooltip
  // re-derives on every minute tick, so it rolls over at midnight
  // instead of freezing at mount.
  const tooltip = $derived.by(() => {
    void time;
    return dateFormatter.format(new Date());
  });
</script>

<Applet appletId="clock" {tooltip} ariaLabel={$t("sh.aria.clock", { weekday, time })}>
  {#snippet labelSnippet()}
    <span class="clock-weekday">{weekday}</span>
    <span class="clock-time">{time}</span>
  {/snippet}
</Applet>

<style>
  .clock-weekday {
    font-size: var(--text-2xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    line-height: 1;
    margin-inline-end: 4px;
  }
  .clock-time {
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--color-fg-shell);
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
</style>
