<script lang="ts">
  import { shellRead } from "$lib/shellRead";
  import { t } from "$lib/i18n/messages";
  /// QS tile: Airplane Mode (rfkill).
  ///
  /// rfkill blocks/unblocks all radios in one go. Toggling here also
  /// affects the WiFi tile and the Bluetooth tile via their respective
  /// refresh listeners.
  import { BaseTile } from "@arlen/ui-kit/components/quicksettings";
  import { Plane } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { shellAction } from "$lib/shellAction";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  /// `null` until rfkill has said. It was `false`, so the tile read "Radios on"
  /// before any answer and forever if none came - a statement about the radios
  /// taken from a store default. `shellRead` already returns null on a failed
  /// read and this leaves the value alone; the initial value was the part that
  /// still asserted.
  let on = $state<boolean | null>(null);

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("arlen://airplane-changed", refresh).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refresh() {
    const got = await shellRead<boolean>("get_airplane_mode", "airplane-tile");
    if (got !== null) on = got;
  }

  async function handleClick() {
    // An unknown state asks to turn airplane mode ON, which is the
    // reversible direction to guess.
    await shellAction("set_airplane_mode", { enabled: on !== true }, "sh.tile.errAirplane");
    // Re-read either way. On success the flip was a guess that happened to be
    // right; on refusal it would have been a guess that was wrong. The rfkill
    // state is the owner, so it answers - and the tile shows what is true rather
    // than what was attempted.
    await refresh();
  }
</script>

<BaseTile
  label={$t("sh.tile.airplane")}
  statusText={on === null
    ? $t("sh.tile.stateUnknown")
    : on
      ? $t("sh.tile.radiosOff")
      : $t("sh.tile.radiosOn")}
  active={on === true}
  onclick={handleClick}
>
  {#snippet icon()}
    <Plane size={16} strokeWidth={1.75} />
  {/snippet}
</BaseTile>
