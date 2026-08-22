<script lang="ts">
  import { shellRead } from "$lib/shellRead";
  import { t } from "$lib/i18n/messages";
  /// Top-bar Airplane Mode badge.
  ///
  /// Visible while rfkill has all radios blocked.
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { Plane } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { shellAction } from "$lib/shellAction";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let active = $state(false);

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("arlen://airplane-changed", refresh).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refresh() {
    const on = await shellRead<boolean>("get_airplane_mode", "airplane");
    if (on !== null) active = on;
  }

  function handleClick() {
    void shellAction("quick_action_run", { id: "qa.toggle_airplane" }, "sh.tile.errAirplane").then(
      () => refresh(),
    );
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  title={$t("sh.badge.airplane")}
  onclick={handleClick}
>
  {#snippet icon()}
    <Plane size={14} strokeWidth={1.75} />
  {/snippet}
</StatusBadge>
