<script lang="ts">
  /// Top-bar Airplane Mode badge.
  ///
  /// Visible while rfkill has all radios blocked.
  import { StatusBadge } from "@lunaris/ui-kit/components/topbar";
  import { Plane } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let active = $state(false);

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("lunaris://airplane-changed", refresh).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refresh() {
    try {
      active = await invoke<boolean>("get_airplane_mode");
    } catch {}
  }

  function handleClick() {
    invoke("quick_action_run", { id: "qa.toggle_airplane" })
      .then(() => refresh())
      .catch(() => {});
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  title="Airplane Mode. Click to disable."
  onclick={handleClick}
>
  {#snippet icon()}
    <Plane size={14} strokeWidth={1.75} />
  {/snippet}
</StatusBadge>
