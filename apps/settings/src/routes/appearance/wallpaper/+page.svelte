<script lang="ts">
  /// The wallpaper picker (wallpaper-plan.md WP-R1): a thumbnail grid of the
  /// available backgrounds, click to set, a fit control, and add-your-own. Static
  /// only for v1; live wallpaper + per-monitor are WP-R2. Fixture-backed under
  /// vite; the daemon bridge (`list_wallpapers`/`set_wallpaper`/`add_wallpaper`)
  /// is a flagged coder seam.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n/messages";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Check, ImagePlus } from "lucide-svelte";
  import {
    wallpapers,
    currentId,
    scale,
    listWallpapers,
    setWallpaper,
    setScale,
    addWallpaper,
    type WallpaperScale,
  } from "$lib/stores/wallpaper";

  onMount(listWallpapers);

  const FITS = $derived<{ value: WallpaperScale; label: string }[]>([
    { value: "fill", label: $t("s.wallpaper.fit.fill") },
    { value: "fit", label: $t("s.wallpaper.fit.fit") },
    { value: "center", label: $t("s.wallpaper.fit.center") },
    { value: "tile", label: $t("s.wallpaper.fit.tile") },
    { value: "stretch", label: $t("s.wallpaper.fit.stretch") },
  ]);
</script>

<Page title={$t("s.wallpaper.title")} description={$t("s.wallpaper.desc")}>
  <SectionGrid>
    <Group label={$t("s.wallpaper.choose")}>
      <div class="wp-inset">
        <div class="wp-grid">
          {#each $wallpapers as w (w.id)}
            <button
              type="button"
              class="wp-tile"
              class:active={$currentId === w.id}
              style="background:{w.thumb}"
              onclick={() => setWallpaper(w.id)}
              aria-pressed={$currentId === w.id}
              aria-label={w.name}
            >
              {#if $currentId === w.id}
                <span class="wp-check" aria-hidden="true"><Check size={13} strokeWidth={2.5} /></span>
              {/if}
              <span class="wp-name">{w.name}</span>
            </button>
          {/each}
        </div>
        <Button variant="outline" size="sm" onclick={addWallpaper}>
          <ImagePlus size={14} strokeWidth={2} aria-hidden="true" />
          {$t("s.wallpaper.add")}
        </Button>
      </div>
    </Group>

    <Group label={$t("s.wallpaper.fit")}>
      <div class="wp-inset">
        <SegmentedControl
          value={$scale}
          options={FITS}
          ariaLabel={$t("s.wallpaper.fit")}
          onchange={(v) => setScale(v as WallpaperScale)}
        />
        <p class="wp-note">{$t("s.wallpaper.comingSoon")}</p>
      </div>
    </Group>
  </SectionGrid>
</Page>

<style>
  /* Group has no inner padding; custom content needs its own inset. */
  .wp-inset {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    padding: 0 1rem 1rem;
  }
  .wp-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: 0.75rem;
  }
  .wp-tile {
    position: relative;
    aspect-ratio: 16 / 10;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card);
    padding: 0;
    cursor: pointer;
    overflow: hidden;
    transition: transform var(--duration-fast, 150ms) ease, box-shadow var(--duration-fast, 150ms) ease;
  }
  .wp-tile:hover {
    transform: translateY(-1px);
  }
  .wp-tile.active {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 2px var(--color-accent);
  }
  .wp-check {
    position: absolute;
    top: 0.4rem;
    inset-inline-end: 0.4rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    border-radius: var(--radius-full);
    background: var(--color-accent);
    color: var(--color-accent-foreground, #fff);
  }
  .wp-name {
    position: absolute;
    inset-inline-start: 0.5rem;
    bottom: 0.4rem;
    font-size: var(--text-2xs);
    font-weight: 500;
    color: #fff;
    text-shadow: 0 1px 3px rgba(0, 0, 0, 0.6);
  }
  .wp-note {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
</style>
