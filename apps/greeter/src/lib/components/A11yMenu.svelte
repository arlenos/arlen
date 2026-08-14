<script lang="ts">
  /// The accessible-login entry in the bottom-left corner (the GDM
  /// "Accessible Login" pattern). The greeter runs before the session, so
  /// these toggles cannot be borrowed from it; they live here and take
  /// effect immediately. High contrast and larger text are pure CSS; the
  /// on-screen keyboard is rendered by the page; the screen-reader toggle
  /// surfaces the hint (the real reader is a deeper, flagged dependency).
  import { Accessibility } from "@lucide/svelte";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import CornerPopover from "./CornerPopover.svelte";
  import {
    a11y,
    toggleA11y,
    screenReaderNotRemembered,
    type A11yState,
  } from "$lib/a11y";
  import { t } from "$lib/i18n/messages";

  // `$derived`, so the labels follow a language switch: a plain const resolves
  // once at module load and keeps whatever was current then.
  const ROWS: { key: keyof A11yState; label: string; id: string }[] = $derived([
    { key: "highContrast", label: $t("g.a11y.highContrast"), id: "greeter-a11y-contrast" },
    { key: "largeText", label: $t("g.a11y.largerText"), id: "greeter-a11y-text" },
    { key: "onScreenKeyboard", label: $t("g.a11y.onScreenKeyboard"), id: "greeter-a11y-osk" },
    { key: "screenReader", label: $t("g.a11y.screenReader"), id: "greeter-a11y-reader" },
  ]);
</script>

<CornerPopover icon={Accessibility} label={$t("g.a11y")} align="left" id="greeter-a11y">
  {#snippet children()}
    <p class="title">{$t("g.a11y")}</p>
    {#each ROWS as row (row.key)}
      <div class="row">
        <span class="label">{row.label}</span>
        <Switch
          value={$a11y[row.key]}
          ariaLabel={row.label}
          onchange={() => toggleA11y(row.key)}
        />
      </div>
    {/each}
    {#if $a11y.screenReader}
      <p class="hint">{$t("g.a11y.readerHint")}</p>
    {/if}
    <!--
      Said beside the switch that made the claim, not in a console. Flipping this
      does two things and only one of them can fail: it applies now (a store
      update) and it is meant to be remembered for the next start (a file write).
      Silence here would let somebody set it once and find it gone at every boot,
      which is the exclusion this whole feature exists to remove.
    -->
    {#if $screenReaderNotRemembered}
      <p class="hint warn">{$t("g.a11y.notRemembered")}</p>
    {/if}
  {/snippet}
</CornerPopover>

<style>
  .title {
    margin: 0;
    padding: 0.25rem 0.5rem 0.375rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    height: var(--height-row, 40px);
    padding: 0 0.5rem;
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
  }
  .row:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .label {
    font-size: calc(0.875rem * var(--greeter-scale, 1));
    color: var(--foreground);
  }
  .hint {
    margin: 0.25rem 0.5rem 0.25rem;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* Full-strength foreground rather than the muted hint colour: this one is not
     background information, it is the difference between a setting that sticks
     and one that has to be found again at every boot. Weight rather than a
     colour, so it still reads under the high-contrast palette. */
  .warn {
    color: var(--foreground);
    font-weight: 500;
  }
</style>
