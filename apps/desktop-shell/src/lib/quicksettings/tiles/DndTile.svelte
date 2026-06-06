<script lang="ts">
  /// QS tile: Do Not Disturb toggle.
  ///
  /// Toggles `dnd_mode` between `off` and `on` via the notifications
  /// daemon. `scheduled` mode is set from the Settings app — clicking
  /// the tile while in scheduled mode flips to `on` (manual override).
  import { BaseTile } from "@lunaris/ui-kit/components/quicksettings";
  import { BellOff, Bell } from "lucide-svelte";
  import { dndState, setDnd } from "$lib/stores/notifications.js";

  /// Always show a subtitle so the icon+label don't collapse to the
  /// top of the tile. "Available" reads better than "Off" because
  /// notifications are still being delivered to the user — only the
  /// suppress filter is inactive.
  const subtitle = $derived(
    $dndState.mode === "off"
      ? "Available"
      : $dndState.mode === "scheduled"
        ? "Scheduled"
        : "Silenced",
  );

  function handleClick() {
    setDnd($dndState.mode === "off" ? "on" : "off");
  }
</script>

<BaseTile
  label="Do Not Disturb"
  statusText={subtitle}
  active={$dndState.mode !== "off"}
  onclick={handleClick}
>
  {#snippet icon()}
    {#if $dndState.mode !== "off"}
      <BellOff size={16} strokeWidth={1.75} />
    {:else}
      <Bell size={16} strokeWidth={1.75} />
    {/if}
  {/snippet}
</BaseTile>
