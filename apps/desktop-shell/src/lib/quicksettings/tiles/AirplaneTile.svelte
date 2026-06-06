<script lang="ts">
  /// QS tile: Airplane Mode (rfkill).
  ///
  /// rfkill blocks/unblocks all radios in one go. Toggling here also
  /// affects the WiFi tile and the Bluetooth tile via their respective
  /// refresh listeners.
  import { BaseTile } from "@lunaris/ui-kit/components/quicksettings";
  import { Plane } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let on = $state(false);

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("lunaris://airplane-changed", refresh).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refresh() {
    try {
      on = await invoke<boolean>("get_airplane_mode");
    } catch {}
  }

  async function handleClick() {
    try {
      await invoke("set_airplane_mode", { enabled: !on });
      on = !on;
    } catch {}
  }
</script>

<BaseTile
  label="Airplane Mode"
  statusText={on ? "Radios off" : "Available"}
  active={on}
  onclick={handleClick}
>
  {#snippet icon()}
    <Plane size={16} strokeWidth={1.75} />
  {/snippet}
</BaseTile>
