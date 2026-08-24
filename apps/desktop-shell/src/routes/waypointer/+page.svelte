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
  // `?searchmock=long` (DEV) answers the app search with more results than the
  // list is tall. That state exists only on a machine with apps installed, so on
  // the VM it could be photographed and nowhere else - which is how a list
  // painting PAST the card's rounded edge stayed an open question: the browser,
  // where a compositing artifact would NOT appear, could not be brought to the
  // same picture. Two renderers, one state, and the difference says whose bug it
  // is.
  if (import.meta.env.DEV && typeof window !== "undefined") {
    if (new URLSearchParams(location.search).get("searchmock") === "long") {
      const app = (name: string, description: string) => ({
        name,
        exec: `arlen-${name.toLowerCase()}`,
        icon_name: "",
        icon_data: null,
        description,
        categories: [],
      });
      (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: async (cmd: string) =>
          cmd === "search_apps"
            ? [
                app("Dateien", "Dateien durchsuchen und ordnen"),
                app("Betrachter", "Ein Bild oder eine Tondatei öffnen"),
                app("E-Mail", "Eine Nachrichtendatei lesen, und was sie über sich selbst sagt"),
                app("Kalender", "Deine eigenen Kalenderdateien, als Agenda"),
                app("Texteditor", "Text lesen und bearbeiten, mit der Herkunft der Datei daneben"),
                app("Wissen", "Deine Zeitleiste, Projekte und was das System dazu aufgezeichnet hat"),
                app("Besprechungen", "Besprechungen aufzeichnen und Notizen behalten"),
                app("Uhr", "Wecker, Timer und Stoppuhr"),
              ]
            : cmd === "evaluate_waypointer_input"
              ? null
              : [],
      };
    }
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
