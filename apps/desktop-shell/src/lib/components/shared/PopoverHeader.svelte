<script lang="ts">
  /// Shared popover header with icon, title, optional switch, and settings button.

  import { Settings } from "lucide-svelte";
  import { Switch } from "$lib/components/ui/switch/index.js";
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
    <Switch
      checked={toggled ?? false}
      onCheckedChange={() => onToggle?.()}
      class="data-checked:bg-[#6366f1]"
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
  .pop-title { flex: 1; font-size: 0.8125rem; font-weight: 500; }
  .pop-settings-btn {
    width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: var(--radius-sm, 4px);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer; padding: 0;
  }
  .pop-settings-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
</style>
