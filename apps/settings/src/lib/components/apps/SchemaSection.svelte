<script lang="ts">
  /// One declared section of an app's settings schema rendered as a Group of
  /// SettingWidget rows. `visible_when` filters live against the current
  /// values; items tagged `advanced` fold behind an expander below the plain
  /// rows (a whole-advanced section becomes one collapsed expander under its
  /// own label). The section description, when declared, reads as a quiet
  /// footnote under the card.
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { ChevronRight } from "lucide-svelte";
  import type { SettingsSection } from "$lib/appSettings";
  import { orderedItems, isVisible } from "$lib/appSettings";
  import SettingWidget from "./SettingWidget.svelte";
  import { t } from "$lib/i18n/messages";

  let {
    appId,
    section,
    values,
    userSet,
    unavailable,
    errors,
  }: {
    appId: string;
    section: SettingsSection;
    values: Record<string, unknown>;
    userSet: string[];
    unavailable: Record<string, string>;
    errors: Record<string, string>;
  } = $props();

  const visibleItems = $derived(orderedItems(section).filter((i) => isVisible(i, values)));
  const plain = $derived(visibleItems.filter((i) => !(i.tags ?? []).includes("advanced")));
  const advanced = $derived(visibleItems.filter((i) => (i.tags ?? []).includes("advanced")));
  const allAdvanced = $derived(plain.length === 0 && advanced.length > 0);
</script>

{#snippet rows(items: typeof visibleItems)}
  {#each items as item (item.key)}
    <SettingWidget
      {appId}
      {item}
      value={values[item.key]}
      userSet={userSet.includes(item.key)}
      unavailableReason={unavailable[item.key]}
      error={errors[item.key]}
    />
  {/each}
{/snippet}

{#if visibleItems.length > 0}
  <div class="sect span-full">
    {#if allAdvanced}
      <Collapsible class="expander">
        <CollapsibleTrigger class="exp-trigger">
          <ChevronRight size={15} strokeWidth={2} />
          {section.label}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <Group>
            {@render rows(advanced)}
          </Group>
        </CollapsibleContent>
      </Collapsible>
    {:else}
      <Group label={section.label}>
        {@render rows(plain)}
      </Group>
      {#if advanced.length > 0}
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            {$t("s.apps.advanced")}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <Group>
              {@render rows(advanced)}
            </Group>
          </CollapsibleContent>
        </Collapsible>
      {/if}
    {/if}
    {#if section.description}
      <p class="sect-desc">{section.description}</p>
    {/if}
  </div>
{/if}

<style>
  .sect {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .sect-desc {
    margin: 0;
    padding-inline-start: 0.25rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  :global(.exp-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: var(--text-sm);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  :global(.exp-trigger:hover) {
    color: var(--foreground);
  }
  :global(.exp-trigger svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.exp-trigger[data-state="open"] svg) {
    transform: rotate(90deg);
  }
</style>
