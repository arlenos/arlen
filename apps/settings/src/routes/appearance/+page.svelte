<script lang="ts">
  /// Appearance landing: the 80%-user hub. Pick a theme, nudge the two or three
  /// high-leverage knobs, step into the six deep pages for full control, and see
  /// everything you have changed in one place. The quick knobs write the SAME
  /// override layer the deep pages do (they are shortcuts, not a separate state).
  ///
  /// Mock-vs-live: the whole area rides the suite's fixture stores + the themes
  /// fixture; nothing persists until the coder wires the suite backend (the quick
  /// knobs and the deep pages share the override layer).
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    Check,
    Upload,
    Sparkles,
    Download,
    Palette,
    Frame,
    Type,
    Zap,
    Terminal,
    LayoutGrid,
    RotateCcw,
  } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import {
    themes,
    activeThemeId,
    loadThemes,
    setActiveTheme,
    installThemeFile,
    importScheme,
    exportTheme,
  } from "$lib/stores/themes";
  import {
    overrides as coloursOv,
    effective as coloursEff,
    isOverridden as isColOv,
    setColorOverride,
    resetColorOverride,
  } from "$lib/stores/themeColors";
  import {
    overrides as geomOv,
    effective as geomEff,
    isOverridden as isGeomOv,
    setGeom,
    resetGeom,
  } from "$lib/stores/themeGeometry";
  import {
    overrides as typoOv,
    effective as typoEff,
    isOverridden as isTypoOv,
    setTypo,
    resetTypo,
  } from "$lib/stores/themeTypography";
  import { overrideSummary, resetAll } from "$lib/stores/themeOverrides";

  onMount(loadThemes);

  const CUSTOMISE = [
    { href: "/appearance/colors", title: "Colours", desc: "The full palette, per role", icon: Palette },
    { href: "/appearance/geometry", title: "Geometry", desc: "Radii, corners, spacing, gaps", icon: Frame },
    { href: "/appearance/typography", title: "Typography", desc: "Fonts, size, weight", icon: Type },
    { href: "/appearance/motion-depth", title: "Motion & Depth", desc: "Speed, easing, shadows", icon: Zap },
    { href: "/appearance/system", title: "System", desc: "Cursor, icons, sounds, terminal", icon: Terminal },
    { href: "/appearance/toolkits", title: "Toolkits", desc: "GTK, Qt, terminal, Wine coverage", icon: LayoutGrid },
  ];

  const accent = $derived(String($coloursEff.accent));
  const roundness = $derived(Math.round(Number($geomEff.intensity) * 100));
  const textSize = $derived(Number($typoEff.sizeBase));
</script>

<Page
  title="Appearance"
  description="Pick a theme, adjust the essentials, or step into the full controls. Everything you change layers on top of the theme, and you can reset it any time."
>
  <SectionGrid>
    <div class="theme-label span-full">Theme</div>
    <div class="grid span-full">
      {#each $themes as t (t.id)}
        {@const active = t.id === $activeThemeId}
        <button type="button" class="theme-card" class:active aria-pressed={active} onclick={() => setActiveTheme(t.id)}>
          <span class="preview" style={`background:${t.swatch[0]}`} aria-hidden="true">
            <span class="pv-window" style={`background:${t.swatch[1]}`}>
              <span class="pv-accent" style={`background:${t.swatch[2]}`}></span>
              <span class="pv-dots">
                <span class="pv-dot" style={`background:${t.swatch[4]}`}></span>
                <span class="pv-dot" style={`background:${t.swatch[3]}`}></span>
              </span>
            </span>
          </span>
          <span class="card-foot">
            <span class="name">{t.name}</span>
            {#if active}<span class="active-mark"><Check size={13} strokeWidth={2.5} /> Active</span>{/if}
          </span>
        </button>
      {/each}
    </div>
    <div class="actions span-full">
      <Button variant="ghost" class="justify-start gap-2 px-3 font-normal text-muted-foreground hover:text-foreground" onclick={() => installThemeFile()}>
        <Upload size={15} strokeWidth={1.75} /> Install a theme file…
      </Button>
      <Button variant="ghost" class="justify-start gap-2 px-3 font-normal text-muted-foreground hover:text-foreground" onclick={() => importScheme("base16")}>
        <Sparkles size={15} strokeWidth={1.75} /> Import a scheme
      </Button>
      <Button variant="ghost" class="justify-start gap-2 px-3 font-normal text-muted-foreground hover:text-foreground" onclick={() => exportTheme()}>
        <Download size={15} strokeWidth={1.75} /> Export current theme
      </Button>
    </div>

    <Group label="Quick adjustments" class="span-full">
      <OverrideRow label="Accent" hint="The primary highlight colour" overridden={isColOv($coloursOv, "accent")} onreset={() => resetColorOverride("accent")} id="quick-accent">
        {#snippet control()}
          <span class="cf">
            <label class="cf-swatch" style={`background:${accent}`} title="Pick an accent">
              <input type="color" value={accent} oninput={(e) => setColorOverride("accent", e.currentTarget.value)} aria-label="Accent colour" />
            </label>
          </span>
        {/snippet}
      </OverrideRow>
      <OverrideRow label="Roundness" hint="How rounded corners are" overridden={isGeomOv($geomOv, "intensity")} onreset={() => resetGeom("intensity")} id="quick-roundness">
        {#snippet control()}
          <ValueSlider value={roundness} min={0} max={200} step={5} unit="%" ariaLabel="Roundness" onchange={(v) => setGeom("intensity", v / 100)} />
        {/snippet}
      </OverrideRow>
      <OverrideRow label="Text size" hint="The base text size" overridden={isTypoOv($typoOv, "sizeBase")} onreset={() => resetTypo("sizeBase")} id="quick-textsize">
        {#snippet control()}
          <ValueSlider value={textSize} min={12} max={18} step={1} unit="px" ariaLabel="Text size" onchange={(v) => setTypo("sizeBase", v)} />
        {/snippet}
      </OverrideRow>
    </Group>

    <div class="cust-label span-full">Customise</div>
    <div class="cust-grid span-full">
      {#each CUSTOMISE as c (c.href)}
        {@const Icon = c.icon}
        <LinkCard href={c.href} title={c.title} description={c.desc}>
          {#snippet icon()}<Icon size={20} strokeWidth={1.75} />{/snippet}
        </LinkCard>
      {/each}
    </div>

    <Group label="My customisations" class="span-full">
      {#if $overrideSummary.total === 0}
        <p class="empty">Nothing changed yet. This lists everything you set on top of the theme.</p>
      {:else}
        {#each $overrideSummary.pages.filter((p) => p.count > 0) as p (p.key)}
          <Row label={p.label} description={`${p.count} change${p.count === 1 ? "" : "s"}`} id={`custo-${p.key}`}>
            {#snippet control()}
              <Button variant="outline" size="sm" onclick={() => goto(p.href)}>Review</Button>
            {/snippet}
          </Row>
        {/each}
        <div class="reset-all">
          <span class="reset-total">{$overrideSummary.total} change{$overrideSummary.total === 1 ? "" : "s"} in all</span>
          <Button variant="outline" size="sm" onclick={() => resetAll()}>
            <RotateCcw size={13} strokeWidth={2} /> Reset all
          </Button>
        </div>
      {/if}
    </Group>
  </SectionGrid>
</Page>

<style>
  .theme-label,
  .cust-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    padding: 0 0.25rem 0.25rem;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(13rem, 1fr));
    gap: 0.75rem;
  }
  .theme-card {
    display: flex;
    flex-direction: column;
    padding: 0;
    text-align: left;
    border-radius: var(--radius-card, 12px);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    overflow: hidden;
    cursor: pointer;
    transition:
      border-color var(--duration-fast, 150ms) var(--ease-out, ease),
      background var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .theme-card:hover {
    border-color: color-mix(in srgb, var(--foreground) 22%, transparent);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .theme-card.active {
    border-color: color-mix(in srgb, var(--color-accent, var(--foreground)) 70%, transparent);
  }
  .theme-card:focus-visible {
    outline: 2px solid var(--color-accent, var(--foreground));
    outline-offset: 2px;
  }
  .preview {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 4.5rem;
  }
  .pv-window {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 68%;
    height: 58%;
    padding: 0 0.5rem;
    border-radius: var(--radius-input, 8px);
  }
  .pv-accent {
    width: 2rem;
    height: 0.6rem;
    border-radius: var(--radius-button, 6px);
  }
  .pv-dots {
    display: inline-flex;
    gap: 0.25rem;
  }
  .pv-dot {
    width: 0.4rem;
    height: 0.4rem;
    border-radius: var(--radius-full, 9999px);
    opacity: 0.85;
  }
  .card-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.625rem 0.75rem;
  }
  .name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .active-mark {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--color-success, #16a34a);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0 0.25rem;
  }

  .cf {
    display: inline-flex;
    align-items: center;
  }
  .cf-swatch {
    position: relative;
    width: 1.75rem;
    height: 1.75rem;
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

  .cust-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(15rem, 1fr));
    gap: 0.5rem;
  }

  .empty {
    margin: 0;
    padding: 0.5rem 1rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .reset-all {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.625rem 1rem;
  }
  .reset-total {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
