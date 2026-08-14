<script lang="ts">
  /// The launcher overlay page. Under vite, `?askmock` (DEV only) seeds Ask mode
  /// with a streamed fixture exchange - the screenshot channel for the quick-ask
  /// pane; `?askmock=off` shows the [ai]-off state. A real boot never hits this:
  /// the query is absent and the block is DEV-gated.
  import { onMount } from "svelte";
  import WaypointerContent from "$lib/components/WaypointerContent.svelte";
  import { askMode, askCapability, askCapabilityLoaded, ask } from "$lib/stores/waypointerAsk";

  // `?searchmock=empty` (DEV) answers every provider with nothing instead of
  // refusing. Under plain vite there is no Tauri at all, so every provider
  // rejects and the launcher's empty line always reads as the refused one -
  // which means the OTHER sentence, the one for a query that genuinely matched
  // nothing, could not be looked at. Two states, two pictures. Runs at module
  // init, before the component's first fan-out.
  if (import.meta.env.DEV && typeof window !== "undefined") {
    if (new URLSearchParams(location.search).get("searchmock") === "empty") {
      // Shape-aware, not one answer for everything: the inline evaluator
      // returns an OPTION, and `[]` is truthy, so a blanket empty array made an
      // empty calculator row render over the very line being looked at.
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: async (cmd: string) =>
          cmd === "evaluate_waypointer_input" ? null : [],
      };
    }
  }

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
