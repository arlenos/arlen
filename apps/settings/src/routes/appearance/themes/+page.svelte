<script lang="ts">
  /// The theme gallery (appearance-surface.md APP-R2): the installed themes as
  /// preview cards, the active one marked, switch by clicking a card, plus
  /// install / import affordances and the cross-toolkit application status. A
  /// theme is one whole look (colours, icons, window style) applied everywhere,
  /// not a light/dark mode.
  ///
  /// Mock-vs-live: the list, previews, install and import read a fixture until
  /// the coder bridges `get_available_themes` / a per-theme palette / the import
  /// commands into settings. Switching persists the active id for real.
  import { onMount } from "svelte";
  import { Upload, Sparkles, Check } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import {
    themes,
    activeThemeId,
    loadThemes,
    setActiveTheme,
    installThemeFile,
    importScheme,
  } from "$lib/stores/themes";

  onMount(loadThemes);

  // The honest cross-toolkit fidelity ceiling (appearance-surface.md section 5).
  // The generators emit to each target; the note states the fidelity there.
  const FIDELITY = [
    { target: "Arlen apps and compositor", note: "Full colour, shape, and every radius" },
    { target: "GTK3", note: "Full shape (adw-gtk3 plus a colour override)" },
    { target: "GTK4 / libadwaita", note: "Colours and the exact accent; the frame is the compositor's" },
    { target: "Qt5 / Qt6", note: "Colour, Fusion-shaped (qt6ct)" },
    { target: "Terminal", note: "The 16-colour ANSI projection" },
    { target: "Icons", note: "The icon set across your GTK and Qt apps" },
  ];
</script>

<Page
  title="Themes"
  description="A theme sets the colours, icons, and window style across your whole desktop and your apps. Pick one, import a community scheme, or install a theme file."
>
  <SectionGrid>
    <div class="grid span-full">
      {#each $themes as t (t.id)}
        {@const active = t.id === $activeThemeId}
        <button
          type="button"
          class="theme-card"
          class:active
          aria-pressed={active}
          onclick={() => setActiveTheme(t.id)}
        >
          <!-- A small mockup of the theme applied: the window ground, a surface
               card with an accent control and a couple of icon dots, so the card
               reads as the whole look rather than a bare colour strip. -->
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
            {#if active}
              <span class="active-mark"><Check size={13} strokeWidth={2.5} /> Active</span>
            {/if}
          </span>
        </button>
      {/each}
    </div>

    <div class="actions span-full">
      <Button
        variant="ghost"
        class="justify-start gap-2 px-3 font-normal text-muted-foreground hover:text-foreground"
        onclick={() => installThemeFile()}
      >
        <Upload size={15} strokeWidth={1.75} />
        Install a theme file…
      </Button>
      <Button
        variant="ghost"
        class="justify-start gap-2 px-3 font-normal text-muted-foreground hover:text-foreground"
        onclick={() => importScheme("base16")}
      >
        <Sparkles size={15} strokeWidth={1.75} />
        Import a scheme (base16 / Catppuccin)
      </Button>
    </div>

    <Group label="Applied to" class="span-full">
      {#each FIDELITY as f (f.target)}
        <Row label={f.target} description={f.note}>
          {#snippet control()}
            <span class="applied"><Check size={14} strokeWidth={2.5} /></span>
          {/snippet}
        </Row>
      {/each}
    </Group>
  </SectionGrid>
</Page>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(13rem, 1fr));
    gap: 0.75rem;
  }

  /* A theme card is the click target: clicking it makes the theme active. Flat
     (hairline border, no shadow) to match the house style; the active card
     takes an accent border, and the others lift on hover. */
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

  /* The mockup preview. */
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
    /* Ride the roundness scale, concentric under the card: the surface takes the
       input radius (a step down from the card), the accent takes the button
       radius, the dots are full. So the preview tracks the roundness setting. */
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
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .active-mark {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--color-success, #16a34a);
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0 0.25rem;
  }
  .applied {
    display: inline-flex;
    color: var(--color-success, #16a34a);
  }
</style>
