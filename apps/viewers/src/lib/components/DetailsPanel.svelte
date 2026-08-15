<script lang="ts">
  /// The details panel, opened with `I` and closed the same way.
  ///
  /// `quickview-plan.md` calls this "the full metadata panel (EXIF for image,
  /// tags/codec for audio, format/streams for video)". It shows the part of that
  /// which is actually measured today - the audio probe reads codec, sample rate,
  /// channels, duration and tags; the image decode reports real dimensions; a
  /// stat gives size and modification time - and it shows NOTHING for the rest.
  ///
  /// There is no EXIF parser in this app, so there is no EXIF section. A panel
  /// that listed "Camera -" and "Exposure -" would say the picture carried no
  /// EXIF, which is a claim nobody here can make. When a parser exists its rows
  /// join this list; until then their absence is the honest report.
  import { t } from "$lib/i18n/messages";
  import { X } from "@lucide/svelte";
  import Button from "@arlen/ui-kit/components/ui/button/button.svelte";

  /// A measured fact. `value` is already formatted for reading; a row whose value
  /// is null is not rendered at all, because "unknown" and "absent" are different
  /// and neither is worth a dash.
  export type Fact = { label: string; value: string | null };

  let { facts, onclose }: { facts: Fact[]; onclose?: () => void } = $props();

  let known = $derived(facts.filter((f) => f.value !== null && f.value !== ""));
</script>

<aside class="panel" aria-label={$t("v.detailsTitle")}>
  <header>
    <h2>{$t("v.detailsTitle")}</h2>
    <Button variant="ghost" size="icon-sm" aria-label={$t("v.close")} onclick={() => onclose?.()}>
      <X class="size-[16px]" strokeWidth={2} />
    </Button>
  </header>
  <dl>
    {#each known as fact (fact.label)}
      <div class="row">
        <dt>{fact.label}</dt>
        <dd>{fact.value}</dd>
      </div>
    {/each}
  </dl>
</aside>

<style>
  /* Plain px and `var(--token, fallback)`, which is the idiom the rest of this
     viewer's chrome uses. The first cut reached for `--space-4` and `--popover`;
     neither is defined in this window, so `right` and `width` were invalid, fell
     back to auto, and the panel laid itself out past the right edge - present in
     the DOM with every row correct, and not on screen. A DOM probe called that
     working. The screenshot did not. */
  .panel {
    position: absolute;
    /* Below the window controls, not over them. At 16px the panel covered the
       minimise and close buttons, so opening details took the window's own
       chrome away - visible the moment it was rendered. */
    top: 52px;
    right: 16px;
    z-index: 20;
    width: min(340px, calc(100vw - 32px));
    max-height: calc(100vh - 32px);
    overflow-y: auto;
    padding: 10px 14px 14px;
    border-radius: var(--radius-card, 12px);
    border: 1px solid color-mix(in srgb, var(--color-fg-primary, #fafafa) 12%, transparent);
    background: color-mix(in srgb, #141414 88%, transparent);
    backdrop-filter: blur(12px);
    color: var(--color-fg-primary, #fafafa);
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.4);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 8px;
  }
  h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
  }
  dl {
    margin: 0;
    display: grid;
    gap: 4px;
  }
  .row {
    display: grid;
    grid-template-columns: 108px 1fr;
    gap: 8px;
    font-size: 13px;
  }
  dt {
    color: color-mix(in srgb, var(--color-fg-primary, #fafafa) 55%, transparent);
  }
  dd {
    margin: 0;
    /* A path or a long tag wraps rather than pushing the panel wider. */
    overflow-wrap: anywhere;
  }
</style>
