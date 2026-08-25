<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// QS tile: Do Not Disturb toggle.
  ///
  /// Toggles `dnd_mode` between `off` and `on` via the notifications
  /// daemon. `scheduled` mode is set from the Settings app — clicking
  /// the tile while in scheduled mode flips to `on` (manual override).
  import { BaseTile } from "@arlen/ui-kit/components/quicksettings";
  import { BellOff, Bell } from "lucide-svelte";
  import { dndState, setDnd } from "$lib/stores/notifications.js";

  /// Always show a subtitle so the icon+label don't collapse to the
  /// top of the tile. "Available" reads better than "Off" because
  /// notifications are still being delivered to the user — only the
  /// suppress filter is inactive.
  const subtitle = $derived(
    $dndState.mode === "unknown"
      ? $t("sh.tile.stateUnknown")
      : $dndState.mode === "off"
        ? $t("sh.tile.available")
        : $dndState.mode === "scheduled"
          ? $t("sh.tile.scheduled")
          : $t("sh.tile.silenced"),
  );

  /// An unknown state is treated as OFF for the press, deliberately: the tile
  /// still has to do something, and asking to silence notifications is the
  /// harmless direction to guess when the daemon has not said which way it is.
  function handleClick() {
    setDnd($dndState.mode === "on" || $dndState.mode === "scheduled" ? "off" : "on");
  }
</script>

<BaseTile
  label={$t("sh.tile.dnd")}
  statusText={subtitle}
  active={$dndState.mode === "on" || $dndState.mode === "scheduled"}
  onclick={handleClick}
>
  {#snippet icon()}
    <!-- Known-on, not not-off: an unknown state draws the ordinary bell rather
         than the crossed-out one, which would claim notifications are silenced. -->
    {#if $dndState.mode === "on" || $dndState.mode === "scheduled"}
      <BellOff size={16} strokeWidth={1.75} />
    {:else}
      <Bell size={16} strokeWidth={1.75} />
    {/if}
  {/snippet}
</BaseTile>
