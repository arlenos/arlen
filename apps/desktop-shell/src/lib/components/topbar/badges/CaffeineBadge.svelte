<script lang="ts">
  /// Top-bar Caffeine badge.
  ///
  /// Visible only while caffeine is on. Click toggles via the
  /// quick-actions dispatcher so the toast pipeline confirms the
  /// state change.
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { Coffee } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  let active = $state(false);

  onMount(() => {
    refresh();
    const interval = setInterval(refresh, 4_000);
    return () => clearInterval(interval);
  });

  async function refresh() {
    try {
      const s = await invoke<{ caffeineActive: boolean }>("get_toggle_status");
      active = s.caffeineActive;
    } catch {}
  }

  function handleClick() {
    invoke("quick_action_run", { id: "qa.toggle_caffeine" })
      .then(() => refresh())
      .catch(() => {});
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  title="Caffeine on. Click to disable."
  onclick={handleClick}
>
  {#snippet icon()}
    <Coffee size={14} strokeWidth={1.75} />
  {/snippet}
</StatusBadge>
