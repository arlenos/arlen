<script lang="ts">
  /// The power control in the bottom-right corner: suspend, restart, shut
  /// down. The backend call is the coder's (org.arlen.Power1 or the same
  /// systemctl/loginctl the shell power plugin uses); this renders the menu
  /// and reports the chosen action.
  import { Power, Moon, RotateCcw, PowerOff } from "@lucide/svelte";
  import CornerPopover from "./CornerPopover.svelte";
  import type { PowerAction } from "$lib/greeter";

  let { onaction }: { onaction: (a: PowerAction) => void } = $props();

  const ITEMS: { action: PowerAction; label: string; icon: typeof Moon; id: string }[] = [
    { action: "suspend", label: "Suspend", icon: Moon, id: "greeter-power-suspend" },
    { action: "reboot", label: "Restart", icon: RotateCcw, id: "greeter-power-reboot" },
    { action: "power-off", label: "Shut down", icon: PowerOff, id: "greeter-power-off" },
  ];
</script>

<CornerPopover icon={Power} label="Power" align="right" id="greeter-power">
  {#snippet children(close: () => void)}
    {#each ITEMS as it (it.action)}
      <button
        type="button"
        class="item"
        id={it.id}
        role="menuitem"
        onclick={() => {
          close();
          onaction(it.action);
        }}
      >
        <it.icon size={16} strokeWidth={1.75} />
        <span>{it.label}</span>
      </button>
    {/each}
  {/snippet}
</CornerPopover>

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    width: 100%;
    height: var(--height-row, 40px);
    padding: 0 0.625rem;
    border: none;
    border-radius: var(--radius-button);
    background: transparent;
    color: var(--foreground);
    font-size: calc(0.875rem * var(--greeter-scale, 1));
    text-align: left;
  }
  .item:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
