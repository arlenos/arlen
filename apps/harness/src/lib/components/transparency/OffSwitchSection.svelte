<script lang="ts">
  /// The off switch, co-located: the answer to "what can it touch" and the
  /// answer to "how do I make it touch nothing" are one gesture apart. The
  /// master switch itself lives in Settings (settings-app.md §0.3); this
  /// states the removability guarantee and links to it. The link opens the
  /// Settings app at its AI section through a host launch command.
  import { t } from "$lib/i18n/messages";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import type { Capability } from "$lib/capability";
  import { openAiSettings } from "$lib/transparency";

  let { capability }: { capability: Capability | null } = $props();

  const off = $derived(capability !== null && !capability.enabled);
</script>

<div class="off">
  <p class="copy">
    You are always in control. You can turn the AI off completely in Settings.
    When it is off, it does nothing at all, and you can remove it. The rest of
    your desktop keeps working without it.
  </p>
  {#if off}
    <p class="status">{$t("h.offswitch.off")}</p>
  {/if}
  <Button id="transparency-offswitch-open" variant="default" size="sm" onclick={openAiSettings}>
    {off ? $t("h.offswitch.turnOn") : $t("h.offswitch.openSettings")}
  </Button>
</div>

<style>
  .off {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.625rem;
    padding: 0.75rem var(--space-row, 0.75rem);
  }
  .copy {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.55;
    color: var(--foreground);
    max-width: 64ch;
  }
  .status {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
