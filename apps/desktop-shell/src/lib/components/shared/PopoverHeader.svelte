<script lang="ts">
  /// Shared popover header with icon, title, optional toggle, and settings button.

  import { Settings } from "lucide-svelte";
  import { closePopover } from "$lib/stores/activePopover.js";
  import Switch from "@arlen/ui-kit/components/ui/switch/switch.svelte";

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
    <Switch
      value={toggled ?? false}
      onchange={() => onToggle?.()}
      ariaLabel="{title} toggle"
    />
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
  /* Title truncates instead of wrapping, so long device names
     (e.g., bluetooth audio) don't push the toggle/settings off
     the right edge or expand the popover height. */
  .pop-title {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pop-settings-btn {
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    padding: 0;
    flex-shrink: 0;
  }
  .pop-settings-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
</style>
