<script lang="ts">
  /// The theme gallery (appearance-surface.md APP-R2): the installed themes as
  /// preview cards, the active one marked, switch with one click, plus install /
  /// import affordances and the cross-toolkit application status. A theme is one
  /// palette applied everywhere (no light/dark mode).
  ///
  /// Mock-vs-live: the list + swatch previews + install + import read a fixture
  /// until the coder bridges `get_available_themes` / a per-theme palette / the
  /// import commands into settings. Switching persists the active id for real.
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

  // The honest cross-toolkit fidelity ceiling (appearance-surface.md §5). The
  // generators emit to each target; the note states the fidelity there.
  const FIDELITY = [
    { target: "Arlen apps + compositor", note: "Full — colour, shape, every radius" },
    { target: "GTK3", note: "Full shape (adw-gtk3 + colour override)" },
    { target: "GTK4 / libadwaita", note: "Colours + exact accent; the frame is the compositor's" },
    { target: "Qt5 / Qt6", note: "Colour, Fusion-shaped (qt6ct)" },
    { target: "Terminal", note: "The 16-colour ANSI projection" },
  ];
</script>

<Page
  title="Themes"
  description="Pick the look of your desktop. A theme is one palette, applied everywhere. Bring your own by importing a scheme or installing a theme file."
>
  <SectionGrid>
    <div class="grid span-full">
      {#each $themes as t (t.id)}
        <div class="theme-card" class:active={t.id === $activeThemeId}>
          <span class="swatch" aria-hidden="true">
            {#each t.swatch as c (c)}<span class="chip" style={`background:${c}`}></span>{/each}
          </span>
          <div class="card-foot">
            <span class="name">{t.name}</span>
            {#if t.id === $activeThemeId}
              <span class="active-mark"><Check size={13} strokeWidth={2.5} /> Active</span>
            {:else}
              <Button variant="outline" size="sm" onclick={() => setActiveTheme(t.id)}>Use</Button>
            {/if}
          </div>
        </div>
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

  /* A theme card: the palette band on top, the name + action below. Flat
     (hairline border, no shadow) to match the house style; the active card
     takes an accent border. */
  .theme-card {
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-card, 12px);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    overflow: hidden;
  }
  .theme-card.active {
    border-color: color-mix(in srgb, var(--color-accent, var(--foreground)) 70%, transparent);
  }
  .swatch {
    display: flex;
    height: 3.5rem;
  }
  .chip {
    flex: 1;
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
