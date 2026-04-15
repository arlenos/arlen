<script lang="ts">
  /// Shared popover header with icon, title, optional toggle, and settings button.

  import { Settings } from "lucide-svelte";
  import { closePopover } from "$lib/stores/activePopover.js";

  interface Props {
    icon: any;
    title: string;
    onSettings?: () => void;
    toggled?: boolean;
    onToggle?: () => void;
  }

  let { icon: Icon, title, onSettings, toggled, onToggle }: Props = $props();
</script>

<div class="pop-header">
  <Icon size={16} strokeWidth={1.5} />
  <span class="pop-title">{title}</span>
  {#if onToggle !== undefined}
    <button
      class="header-toggle"
      class:active={toggled ?? false}
      onclick={(e) => { e.stopPropagation(); onToggle?.(); }}
      role="switch"
      aria-checked={toggled ?? false}
      aria-label="{title} toggle"
    >
      <span class="header-toggle-thumb"></span>
    </button>
  {/if}
  <button
    class="pop-settings-btn"
    onclick={(e) => { e.stopPropagation(); onSettings ? onSettings() : closePopover(); }}
    title="Settings"
  >
    <Settings size={14} strokeWidth={1.5} />
  </button>
</div>

<style>
  .pop-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
  .pop-title { flex: 1; font-size: 0.8125rem; font-weight: 500; }
  .pop-settings-btn {
    width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: var(--radius-sm);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer; padding: 0;
  }
  .pop-settings-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }

  /* Custom toggle - matches shell design system, no shadcn */
  .header-toggle {
    position: relative; width: 36px; height: 20px; border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border: none; cursor: pointer; padding: 0; flex-shrink: 0;
    transition: background-color 150ms ease;
  }
  .header-toggle:hover { background: color-mix(in srgb, var(--color-fg-shell) 30%, transparent); }
  .header-toggle.active { background: color-mix(in srgb, var(--color-accent) 60%, transparent); }
  .header-toggle-thumb {
    position: absolute; top: 2px; left: 2px;
    width: 16px; height: 16px; border-radius: var(--radius-md);
    background: var(--color-fg-shell);
    transition: transform 150ms ease;
  }
  .header-toggle.active .header-toggle-thumb { transform: translateX(16px); }
</style>
