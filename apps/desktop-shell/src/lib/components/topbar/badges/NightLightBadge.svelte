<script lang="ts">
  /// Top-bar Night Light badge.
  ///
  /// Visible while the warm-tint compositor effect is active. Reads
  /// the shell-config `night_light.enabled` flag, mirrors the
  /// `lunaris://shell-config-changed` event for live updates.
  import { StatusBadge } from "@lunaris/ui-kit/components/topbar";
  import { Sunset } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  interface ShellConfig {
    night_light: { enabled: boolean };
  }

  let active = $state(false);

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("lunaris://shell-config-changed", refresh).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refresh() {
    try {
      const c = await invoke<ShellConfig>("get_shell_config");
      active = c.night_light?.enabled ?? false;
    } catch {}
  }

  function handleClick() {
    invoke("quick_action_run", { id: "qa.toggle_night_light" })
      .then(() => refresh())
      .catch(() => {});
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  title="Night Light on. Click to disable."
  onclick={handleClick}
>
  {#snippet icon()}
    <Sunset size={14} strokeWidth={1.75} />
  {/snippet}
</StatusBadge>
