<script lang="ts">
  /// Colours: the theme's palette, editable per role. The common roles carry the
  /// everyday look; the full 18 sit behind an expander. Each role is an
  /// override-row (resolved value at full contrast; an accent bar + reset when
  /// overridden). A live preview strip and a WCAG contrast check update as you
  /// edit. Rich by structure, not by omission (appearance-surface.md).
  ///
  /// Mock-vs-live: reads a fixture palette + holds overrides in the store until
  /// the coder exposes the resolved per-role palette + the per-field override
  /// writes (theme.toml layer 3). Affordance-only until then.
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import ThemePreview from "$lib/components/appearance/ThemePreview.svelte";
  import {
    COLOR_ROLES,
    resolved,
    overrides,
    effective,
    isOverridden,
    setColorOverride,
    resetColorOverride,
    contrastRatio,
    CONTRAST_PAIRS,
    isHex,
    normHex,
    type ColorRole,
  } from "$lib/stores/themeColors";

  const common = COLOR_ROLES.filter((r) => r.tier === "common");
  const full = COLOR_ROLES.filter((r) => r.tier === "full");

  function onHexInput(role: ColorRole, value: string) {
    if (isHex(value)) setColorOverride(role.key, normHex(value));
  }
</script>

<Page
  title="Colours"
  description="The theme's palette. Each colour shows the active theme's value; change one and it overrides just that role, on top of the theme."
>
  <SectionGrid>
    <div class="preview-wrap span-full">
      <ThemePreview colors={$effective} />
    </div>

    <Group label="Contrast" class="span-full">
      {#each CONTRAST_PAIRS as pair (pair.label)}
        {@const ratio = contrastRatio($effective[pair.fg], $effective[pair.bg])}
        {@const pass = ratio >= 4.5}
        <div class="contrast-row">
          <span class="contrast-label">{pair.label}</span>
          <span class="contrast-value">
            <span class="contrast-num">{ratio.toFixed(1)}:1</span>
            <Badge variant={pass ? "success" : "warn"}>{pass ? "Passes" : "Low"}</Badge>
          </span>
        </div>
      {/each}
    </Group>

    <Group label="Colours" class="span-full">
      {#each common as role (role.key)}
        <OverrideRow
          label={role.label}
          hint={role.hint}
          overridden={isOverridden($overrides, role.key)}
          onreset={() => resetColorOverride(role.key)}
          id={`color-${role.key}`}
        >
          {#snippet control()}
            {@render colorControl(role)}
          {/snippet}
        </OverrideRow>
      {/each}
    </Group>

    <Collapsible class="span-full all-roles">
      <CollapsibleTrigger class="all-trigger">
        <ChevronRight size={15} strokeWidth={2} />
        All roles
      </CollapsibleTrigger>
      <CollapsibleContent>
        <Group class="span-full">
          {#each full as role (role.key)}
            <OverrideRow
              label={role.label}
              hint={role.hint}
              overridden={isOverridden($overrides, role.key)}
              onreset={() => resetColorOverride(role.key)}
              id={`color-${role.key}`}
            >
              {#snippet control()}
                {@render colorControl(role)}
              {/snippet}
            </OverrideRow>
          {/each}
        </Group>
      </CollapsibleContent>
    </Collapsible>
  </SectionGrid>
</Page>

<!-- The swatch + hex editor for one role. The swatch opens the platform colour
     chooser; the hex field takes a precise value. -->
{#snippet colorControl(role: ColorRole)}
  {@const val = $effective[role.key]}
  <span class="cf">
    <label class="cf-swatch" style={`background:${val}`} title="Pick a colour">
      <input
        type="color"
        value={val}
        oninput={(e) => setColorOverride(role.key, e.currentTarget.value)}
        aria-label={`${role.label} colour`}
      />
    </label>
    <input
      class="cf-hex"
      type="text"
      value={val}
      spellcheck="false"
      aria-label={`${role.label} hex value`}
      onchange={(e) => onHexInput(role, e.currentTarget.value)}
    />
  </span>
{/snippet}

<style>
  .preview-wrap {
    /* Sticky so edits stay visible while scrolling the roles. The opaque page
       background + padding make it a clean pinned bar that masks the content
       scrolling under it, rather than letting it bleed through. */
    position: sticky;
    top: 0;
    z-index: 3;
    background: var(--color-bg-app);
    padding-bottom: 0.75rem;
  }

  /* The "All roles" disclosure trigger (the class rides the Collapsible root, so
     these are global) + its rotating chevron. */
  :global(.all-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: 0.8125rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  :global(.all-trigger:hover) {
    color: var(--foreground);
  }
  :global(.all-trigger svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.all-trigger[data-state="open"] svg) {
    transform: rotate(90deg);
  }

  .contrast-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.4375rem 1rem;
  }
  .contrast-label {
    font-size: 0.8125rem;
    color: var(--foreground);
  }
  .contrast-value {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .contrast-num {
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* The colour field: a swatch that opens the picker + a hex input. */
  .cf {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .cf-swatch {
    position: relative;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--radius-button, 6px);
    border: 1px solid color-mix(in srgb, var(--foreground) 18%, transparent);
    cursor: pointer;
    overflow: hidden;
  }
  .cf-swatch input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .cf-hex {
    width: 6rem;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
    padding: 0.3125rem 0.5rem;
    border-radius: var(--radius-input, 8px);
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 14%, transparent);
    color: var(--foreground);
    outline: none;
  }
  .cf-hex:focus {
    border-color: color-mix(in srgb, var(--color-accent, var(--foreground)) 60%, transparent);
  }
</style>
