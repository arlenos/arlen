<script lang="ts">
  /// The launcher overlay page. Under vite, `?askmock` (DEV only) seeds Ask mode
  /// with a streamed fixture exchange - the screenshot channel for the quick-ask
  /// pane; `?askmock=off` shows the [ai]-off state. A real boot never hits this:
  /// the query is absent and the block is DEV-gated.
  import { onMount } from "svelte";
  import WaypointerContent from "$lib/components/WaypointerContent.svelte";
  import { askMode, askCapability, askCapabilityLoaded, ask } from "$lib/stores/waypointerAsk";

  onMount(() => {
    if (!import.meta.env.DEV) return;
    const mock = new URLSearchParams(location.search).get("askmock");
    if (mock === null) return;
    askMode.set(true);
    if (mock === "off") {
      askCapability.set({ enabled: false, tier: "None", actionMode: "Suggest", executorLive: false });
      askCapabilityLoaded.set(true);
      return;
    }
    askCapability.set({
      enabled: true,
      tier: "Project",
      actionMode: "Suggest",
      provider: "ollama-default",
      model: "qwen2.5:7b",
      executorLive: true,
    });
    askCapabilityLoaded.set(true);
    void ask("whats my battery policy");
  });
</script>

<WaypointerContent />
