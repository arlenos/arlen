<script lang="ts">
  /// One waypointer result row: the leading visual (plain lucide
  /// icon, app-icon image with optional corner badge, or a raw text
  /// glyph), title + optional description, and an optional trailing
  /// slot. Lives INSIDE the host's Command.Item — selection values
  /// and onSelect handlers stay with the host; this is purely the
  /// row anatomy all thirteen groups used to hand-roll.
  import type { Snippet } from "svelte";
  import { AppWindow, Skull } from "lucide-svelte";

  let {
    icon,
    iconUrl = null,
    fallbackIcon,
    badge,
    glyph,
    emphasis = 70,
    title,
    description = null,
    trailing,
  }: {
    /// Plain lucide leading icon (16px register).
    icon?: typeof AppWindow;
    /// App icon image (20px register). Rows that may carry one pass
    /// `fallbackIcon` for the muted stand-in when the url is null.
    iconUrl?: string | null;
    fallbackIcon?: typeof AppWindow;
    /// Corner badge over the app icon: the window marker or the
    /// kill-mode skull.
    badge?: "window" | "kill";
    /// Raw text glyph leading slot (unicode results).
    glyph?: string;
    /// Muting tier for the plain lucide icon: 70 for actionable
    /// rows, 60 for passive/informational ones.
    emphasis?: 60 | 70;
    title: string;
    description?: string | null;
    /// Trailing inline content (clipboard delete, settings inline
    /// control). Its presence makes the text column grow so the
    /// trailing content pins to the right edge.
    trailing?: Snippet;
  } = $props();
</script>

{#if glyph != null}
  <span class="wp-unicode-char">{glyph}</span>
{:else if iconUrl}
  {#if badge}
    <span class="wp-win-icon-wrap">
      <img src={iconUrl} alt="" class="wp-app-icon" />
      <span class="wp-win-badge" class:wp-kill-badge={badge === "kill"}>
        {#if badge === "kill"}
          <Skull size={8} strokeWidth={2} />
        {:else}
          <AppWindow size={8} strokeWidth={2} />
        {/if}
      </span>
    </span>
  {:else}
    <img src={iconUrl} alt="" class="wp-app-icon" />
  {/if}
{:else if fallbackIcon}
  {@const Fallback = fallbackIcon}
  <Fallback size={16} strokeWidth={1.5} class="wp-fallback-icon" />
{:else if icon}
  {@const Icon = icon}
  <Icon
    size={16}
    strokeWidth={1.5}
    class="shrink-0 {emphasis === 70 ? 'opacity-70' : 'opacity-60'}"
  />
{/if}
<div class="wp-app-info" class:wp-app-info-grow={!!trailing}>
  <span class="wp-app-name">{title}</span>
  {#if description}
    <span class="wp-app-desc">{description}</span>
  {/if}
</div>
{#if trailing}
  {@render trailing()}
{/if}

<style>
  .wp-win-icon-wrap {
    position: relative;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
  }

  .wp-win-badge {
    position: absolute;
    bottom: -3px;
    right: -3px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    background: var(--color-bg-shell);
    border-radius: var(--radius-chip);
    color: var(--color-fg-shell);
    opacity: 0.7;
  }

  .wp-kill-badge {
    color: var(--color-error);
    opacity: 0.9;
  }

  .wp-unicode-char {
    font-size: var(--text-xl);
    line-height: 1;
    width: var(--height-control-compact, 24px);
    text-align: center;
    flex-shrink: 0;
  }

  .wp-app-icon {
    width: 20px;
    height: 20px;
    border-radius: var(--radius-chip);
    object-fit: contain;
    flex-shrink: 0;
  }

  /* The lucide svg renders outside this component's scope hash. */
  :global(.wp-fallback-icon) {
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    opacity: 0.4;
  }

  .wp-app-info {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .wp-app-info-grow {
    flex: 1;
  }

  .wp-app-name {
    font-size: var(--text-sm);
    line-height: 1.3;
  }

  .wp-app-desc {
    font-size: var(--text-2xs);
    line-height: 1.3;
    opacity: 0.45;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
