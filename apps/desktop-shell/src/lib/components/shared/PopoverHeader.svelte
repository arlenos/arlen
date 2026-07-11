<script lang="ts">
  /// Shared popover header with icon, title, optional toggle, and an
  /// optional settings shortcut.
  ///
  /// The gear renders only when the consumer provides `onSettings`
  /// (the deep-link into the matching Settings panel). It used to
  /// fall back to closing the popover, which made a button labelled
  /// Settings silently act as Close — once Settings grows the device
  /// panels, consumers pass the real handler and the gear returns.

  import { Settings } from "lucide-svelte";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import Switch from "@arlen/ui-kit/components/ui/switch/switch.svelte";

  interface Props {
    /// A lucide icon component; they all share one signature, so
    /// any concrete one types the slot.
    icon: typeof Settings;
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
      ariaLabel="Turn {title} on or off"
    />
  {/if}
  {#if onSettings}
    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <button
            {...props}
            class="pop-settings-btn"
            aria-label="{title} settings"
            onclick={(e) => { e.stopPropagation(); onSettings(); }}
          >
            <Settings size={14} strokeWidth={1.5} />
          </button>
        {/snippet}
      </Tooltip.Trigger>
      <Tooltip.TooltipContent side="bottom">Settings</Tooltip.TooltipContent>
    </Tooltip.Root>
  {/if}
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
    font-size: var(--text-sm);
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
