<script lang="ts">
  /// The wallpaper picker (wallpaper-plan.md WP-R1): a thumbnail grid of the
  /// available backgrounds, click to set, a fit control, and add-your-own. Static
  /// only for v1; live wallpaper + per-monitor are WP-R2. The daemon bridge
  /// (`list_wallpapers`/`set_wallpaper`/`add_wallpaper`) is BUILT; this note
  /// called it a flagged seam afterwards. Fixture-backed under vite, where there
  /// is no host to answer.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n/messages";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { SwatchGrid, Swatch } from "@arlen/ui-kit/components/ui/swatch-grid";
  import { ImagePlus } from "lucide-svelte";
  import {
    wallpapers,
    wallpapersUnavailable,
    wallpaperChangeFailed,
    currentId,
    scale,
    listWallpapers,
    setWallpaper,
    setScale,
    addWallpaper,
    type WallpaperScale,
  } from "$lib/stores/wallpaper";

  onMount(listWallpapers);

  // Two, because the daemon renders two. `daemon_scale` in
  // `src-tauri/src/commands/wallpaper.rs` maps fill and fit and refuses the rest
  // by name, on purpose - stretch breaks aspect and mapping it onto fill would
  // show the user something other than what they picked. That refusal was right
  // and the picker offered all five anyway, so Center, Tile and Stretch were
  // three buttons whose only outcome was an error. A control that cannot work
  // does not appear; when the daemon learns a fit, it comes back here.
  const FITS = $derived<{ value: WallpaperScale; label: string }[]>([
    { value: "fill", label: $t("s.wallpaper.fit.fill") },
    { value: "fit", label: $t("s.wallpaper.fit.fit") },
  ]);
</script>

<Page back={{ href: "/appearance", label: $t("s.nav.appearance") }} title={$t("s.wallpaper.title")} description={$t("s.wallpaper.desc")}>
  <SectionGrid>
    <Section label={$t("s.wallpaper.choose")}>
      <div class="wp-inset">
        {#if $wallpapersUnavailable}
          <!-- Empty here would read as "no wallpapers installed", which is never
               true and is not what happened. Inside the inset, not above it:
               `.wp-note` has no padding of its own because the inset provides
               it, so placing it outside drew the text across the box's border. -->
          <p class="wp-note">{$t("s.wallpaper.unavailable")}</p>
        {/if}
        <!-- The highlight is back on the wallpaper that is actually up. -->
        {#if $wallpaperChangeFailed}
          <p class="wp-note" role="alert">{$t("s.wallpaper.changeFailed")}</p>
        {/if}
        <SwatchGrid min="8rem" label={$t("s.wallpaper.choose")}>
          {#each $wallpapers as w (w.id)}
            <Swatch label={w.name} color={w.thumb} active={$currentId === w.id} onclick={() => setWallpaper(w.id)} />
          {/each}
        </SwatchGrid>
        <Button variant="outline" size="sm" onclick={addWallpaper}>
          <ImagePlus size={14} strokeWidth={2} aria-hidden="true" />
          {$t("s.wallpaper.add")}
        </Button>
      </div>
    </Section>

    <Section label={$t("s.wallpaper.fit")}>
      <div class="wp-inset">
        <SegmentedControl
          value={$scale}
          options={FITS}
          ariaLabel={$t("s.wallpaper.fit")}
          onchange={(v) => setScale(v as WallpaperScale)}
        />
      </div>
    </Section>
  </SectionGrid>
</Page>

<style>
  /* Group has no inner padding; custom content needs its own inset. Children take
     natural width (left-aligned) so the Add button + the Fit control don't stretch
     across the card; only the grid fills the row. */
  .wp-inset {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1rem;
    padding: 0.75rem 1rem 1.1rem;
  }
  .wp-note {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--color-fg-secondary, #a1a1aa);
  }
</style>
