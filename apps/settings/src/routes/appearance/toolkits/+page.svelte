<script lang="ts">
  /// Toolkits: the honest cross-toolkit surface. One Arlen theme drives GTK / Qt /
  /// Terminal / Wine; this states per toolkit how far it reaches (the fidelity
  /// ceiling), whether it is on, and a per-toolkit override. A flat list, never an
  /// N x M matrix; ragged coverage stated per row (Wine = best-effort). Not the
  /// split editor - a status + control list.
  ///
  /// Mock-vs-live: the coverage tiers + notes are real; the per-toolkit on/off, the
  /// override map, and the prerequisite detection need coder backend. Fixture.
  import { ChevronRight, RotateCcw } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { effective as colorsEffective } from "$lib/stores/themeColors";
  import {
    TOOLKITS,
    coverageBadge,
    disabled,
    accentOverrides,
    isEnabled,
    setEnabled,
    hasAccentOverride,
    setAccentOverride,
    resetAccentOverride,
  } from "$lib/stores/themeToolkits";

  const hubAccent = $derived(String($colorsEffective.accent));
</script>

<Page
  title="Toolkits"
  description="One theme drives every toolkit. Here is how far it reaches on each, whether it is on, and where you can override it. Coverage is ragged and honest: some toolkits take the full look, some only the colours."
>
  <SectionGrid>
    <div class="tk-list span-full">
    {#each TOOLKITS as tk (tk.id)}
      {@const badge = coverageBadge(tk.coverage)}
      {@const on = isEnabled($disabled, tk.id)}
      <div class="tk-card" class:off={!tk.native && !on}>
        <div class="tk-head">
          <div class="tk-title">
            <span class="tk-name">{tk.name}</span>
            <Badge variant={badge.tone}>{badge.label}</Badge>
          </div>
          {#if tk.native}
            <span class="tk-always">Always on</span>
          {:else}
            <Switch value={on} ariaLabel={`Apply the theme to ${tk.name}`} onchange={(v) => setEnabled(tk.id, v)} />
          {/if}
        </div>

        <p class="tk-note">{tk.note}</p>
        {#if tk.prereq}
          <p class="tk-prereq">{tk.prereq}</p>
        {/if}

        {#if !tk.native}
          <Collapsible class="tk-override">
            <CollapsibleTrigger class="ovr-trigger">
              <ChevronRight size={14} strokeWidth={2} />
              Override
              {#if hasAccentOverride($accentOverrides, tk.id)}<span class="ovr-dot"></span>{/if}
            </CollapsibleTrigger>
            <CollapsibleContent>
              <div class="ovr-body">
                <div class="ovr-row">
                  <span class="ovr-label">Accent for {tk.name}</span>
                  <span class="ovr-accent">
                    {#if hasAccentOverride($accentOverrides, tk.id)}
                      <button class="ovr-reset" type="button" aria-label="Reset accent" title="Back to the theme accent" onclick={() => resetAccentOverride(tk.id)}>
                        <RotateCcw size={12} strokeWidth={2} />
                      </button>
                    {/if}
                    <label
                      class="ovr-swatch"
                      style={`background:${$accentOverrides[tk.id] ?? hubAccent}`}
                      title="Pick an accent for this toolkit"
                    >
                      <input
                        type="color"
                        value={$accentOverrides[tk.id] ?? hubAccent}
                        oninput={(e) => setAccentOverride(tk.id, e.currentTarget.value)}
                        aria-label={`Accent for ${tk.name}`}
                      />
                    </label>
                  </span>
                </div>
                <p class="ovr-note">Overrides the theme accent for {tk.name} only. Everything else follows the theme.</p>
              </div>
            </CollapsibleContent>
          </Collapsible>
        {/if}
      </div>
    {/each}
    </div>
  </SectionGrid>
</Page>

<style>
  .tk-list {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    max-width: 44rem;
  }
  .tk-card {
    padding: 0.875rem 1rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    transition: opacity var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .tk-card.off {
    opacity: 0.55;
  }
  .tk-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .tk-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  .tk-name {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .tk-always {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .tk-note {
    margin: 0.375rem 0 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .tk-prereq {
    margin: 0.25rem 0 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
  }

  /* The per-toolkit override disclosure. */
  :global(.ovr-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    margin-top: 0.625rem;
    padding: 0.25rem 0;
    border: none;
    background: transparent;
    font-size: 0.75rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
  }
  :global(.ovr-trigger:hover) {
    color: var(--foreground);
  }
  :global(.ovr-trigger svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.ovr-trigger[data-state="open"] svg:first-child) {
    transform: rotate(90deg);
  }
  .ovr-dot {
    width: 0.375rem;
    height: 0.375rem;
    border-radius: var(--radius-full, 9999px);
    background: var(--color-accent, var(--foreground));
  }
  .ovr-body {
    padding: 0.5rem 0 0.25rem;
  }
  .ovr-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .ovr-label {
    font-size: 0.8125rem;
    color: var(--foreground);
  }
  .ovr-accent {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
  }
  .ovr-reset {
    display: inline-flex;
    border: none;
    background: transparent;
    padding: 0.25rem;
    border-radius: var(--radius-button, 6px);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
  }
  .ovr-reset:hover {
    color: var(--foreground);
  }
  .ovr-swatch {
    position: relative;
    width: 1.5rem;
    height: 1.5rem;
    border-radius: var(--radius-button, 6px);
    border: 1px solid color-mix(in srgb, var(--foreground) 18%, transparent);
    cursor: pointer;
    overflow: hidden;
  }
  .ovr-swatch input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .ovr-note {
    margin: 0.5rem 0 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
