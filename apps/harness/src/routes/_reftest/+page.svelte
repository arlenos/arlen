<script lang="ts">
  /// Headless render harness for the file-reference pills. UI-AFFORDANCE
  /// verification ONLY, NOT a behaviour claim. Mocks the daemon over Tauri IPC
  /// (only when no Tauri runtime is present, so it never hijacks the real app):
  /// the resolvable check marks one path missing; open / reveal / open-with just
  /// log. Renders the real assistant-prose container (markdown + the `fileRefs`
  /// action + the shared right-click menu) so the soft-inline pills, their hover
  /// affordance, the menu and the muted not-found state are screenshot-verifiable.
  /// The real open-as-user + the daemon's ref emission are coder/metal. Dev route.
  import { onMount } from "svelte";
  import { renderMarkdown } from "$lib/markdown";
  import { externalLinks } from "$lib/externalLinks";
  import { fileRefs } from "$lib/fileRefs";
  import FileRefMenu from "$lib/components/chat/FileRefMenu.svelte";

  const MISSING = "/home/tim/arlen/tmp/scratch.rs";

  const text = `I reviewed the parser and made three edits. I tightened the tokenizer in [main.rs](arlenfile:///home/tim/arlen/ai/ai-agent/src/main.rs) and the shared types in [lexer.rs](arlenfile:///home/tim/arlen/ai/ai-core/src/lexer.rs), then refreshed the fixtures in [cases.toml](arlenfile:///home/tim/arlen/tests/cases.toml).

The old [scratch.rs](arlenfile://${MISSING}) is gone now, so I left it alone. The sunset shot [inn.jpg](arlenfile:///home/tim/pictures/inn.jpg) is the one you attached.`;

  let ready = $state(false);
  onMount(async () => {
    // Only mock when there is no real Tauri backend, so this dev route can never
    // hijack the live app's IPC.
    const tauriAvailable = "__TAURI_INTERNALS__" in window;
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd, args) => {
        const a = (args ?? {}) as Record<string, unknown>;
        if (cmd === "fileref_resolve") {
          const paths = (a.paths as string[]) ?? [];
          return paths.map((path) => ({ path, resolvable: path !== MISSING }));
        }
        // open / reveal / open-with: log the intent, no side effect.
        return null;
      });
    }
    ready = true;
  });
</script>

<div class="harness">
  <h2>Assistant message with file references</h2>
  <div class="bubble">
    {#if ready}
      <div class="block prose markdown" use:externalLinks use:fileRefs={text}>
        {@html renderMarkdown(text)}
      </div>
    {/if}
  </div>
  <p class="hint">
    Hover a pill for the affordance; right-click for the menu. The missing path
    reads muted.
  </p>
  <FileRefMenu />
</div>

<style>
  .harness {
    padding: 32px;
    min-height: 100vh;
    background: var(--background);
    color: var(--foreground);
  }
  h2 {
    margin: 0 0 16px;
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .bubble {
    max-width: 44rem;
    font-size: 0.9375rem;
    line-height: 1.6;
  }
  .hint {
    margin-top: 24px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
