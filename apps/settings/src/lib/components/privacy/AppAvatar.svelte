<script lang="ts">
  /// The app identity tile: a calm slot for the icon, forward-compatible with a
  /// real app icon replacing the glyph. Known first-party principals get their
  /// own mark; everything else falls back to an initial tile. A real per-app
  /// icon (the shell's app_index carries one) can replace this once a Settings
  /// bridge exposes it.
  import { Sparkles, FolderOpen, SquareTerminal, SlidersHorizontal } from "lucide-svelte";

  const APP_ICONS: Record<string, typeof Sparkles> = {
    "org.arlen.AI1": Sparkles,
    "ai-daemon": Sparkles,
    "org.arlen.AIAgent1": Sparkles,
    "ai-agent": Sparkles,
    "org.arlen.files": FolderOpen,
    "org.arlen.terminal": SquareTerminal,
    "org.arlen.settings": SlidersHorizontal,
  };

  let { appId, label, size = 28 }: { appId: string; label: string; size?: number } = $props();
  const Icon = $derived(APP_ICONS[appId]);
</script>

<span class="avatar" style={`width:${size}px;height:${size}px`}>
  {#if Icon}
    <Icon size={size * 0.6} strokeWidth={1.75} />
  {:else}
    <span class="avatar-initial" style={`font-size:${size * 0.42}px`}>
      {label.charAt(0).toUpperCase()}
    </span>
  {/if}
</span>

<style>
  .avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .avatar-initial {
    font-weight: 600;
    line-height: 1;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
</style>
