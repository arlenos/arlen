<script lang="ts">
  import { shellRead } from "$lib/shellRead";
  import { t } from "$lib/i18n/messages";
  /// Top-bar Caffeine badge.
  ///
  /// Visible only while caffeine is on. Click toggles via the
  /// quick-actions dispatcher so the toast pipeline confirms the
  /// state change.
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { Coffee } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { shellAction } from "$lib/shellAction";
  import { onMount } from "svelte";

  let active = $state(false);

  onMount(() => {
    refresh();
    const interval = setInterval(refresh, 4_000);
    return () => clearInterval(interval);
  });

  async function refresh() {
    const s = await shellRead<{ caffeineActive: boolean }>("get_toggle_status", "caffeine");
    if (s !== null) active = s.caffeineActive;
  }

  function handleClick() {
    void shellAction("quick_action_run", { id: "qa.toggle_caffeine" }, "sh.badge.errCaffeine").then(
      () => refresh(),
    );
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  title={$t("sh.badge.caffeine")}
  onclick={handleClick}
>
  {#snippet icon()}
    <Coffee size={14} strokeWidth={1.75} />
  {/snippet}
</StatusBadge>
