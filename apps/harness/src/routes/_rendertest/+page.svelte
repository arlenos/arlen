<script lang="ts">
  /// Headless render harness for the artifact widget. UI-AFFORDANCE verification
  /// ONLY (pure-prop components, no daemon). It shows the settled model: each
  /// artifact is placed by kind+size - visual + small text/data render FULL
  /// INLINE; large text/data show a minimal reference card and open in the right
  /// pane (auto-opened here to simulate production). Real emit + Save/Pin are
  /// coder/backend. Not shipped in any nav; dev only.
  import { onMount } from "svelte";
  import ArtifactBlock from "$lib/components/chat/ArtifactBlock.svelte";
  import ArtifactCard from "$lib/components/chat/ArtifactCard.svelte";
  import ArtifactPanel from "$lib/components/ArtifactPanel.svelte";
  import { placement } from "$lib/components/artifact/placement";
  import { openArtifact, openPane, closePane } from "$lib/stores/artifact";
  import type { Artifact } from "$lib/components/artifact/types";

  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="240" height="120"><rect width="240" height="120" fill="#3a3a3a"/><circle cx="80" cy="60" r="34" fill="#8aa"/><text x="150" y="66" font-family="sans-serif" font-size="15" fill="#eee">image</text></svg>';
  const svgB64 = typeof btoa !== "undefined" ? btoa(svg) : "";
  const longCode =
    "def pipeline(items, opts):\n" +
    Array.from({ length: 36 }, (_, i) => `    items[${i}] = transform(items[${i}], opts)  # step ${i + 1}`).join("\n") +
    "\n    return items";

  const artifacts: Artifact[] = [
    // small code -> inline
    {
      kind: "code",
      payload: { kind: "code", language: "python", source: "def add(a, b):\n    return a + b" },
      text: "def add(a, b): return a + b",
      meta: { origin: "agent_generated", title: "add()" },
    },
    // chart -> inline (visual)
    {
      kind: "chart",
      payload: { kind: "chart", chart_type: "bar", series: [{ name: "Reads", values: [4, 9, 6, 12, 8] }, { name: "Writes", values: [2, 3, 5, 4, 7] }] },
      text: "Reads vs Writes",
      meta: { origin: "agent_generated", title: "Activity by day" },
    },
    // small table -> inline
    {
      kind: "table",
      payload: { kind: "table", columns: ["File", "Size"], rows: [["thesis.md", "48 KB"], ["notes.txt", "3 KB"]] },
      text: "files",
      meta: { origin: "agent_generated", title: "Two files" },
    },
    // links -> inline (visual)
    {
      kind: "links",
      payload: { kind: "links", links: [{ href: "https://arlenos.org", label: "Arlen OS" }, { href: "javascript:alert(1)", label: "Blocked" }] },
      text: "links",
      meta: { origin: "external_content", title: "References" },
    },
    // big code -> pane (card in chat)
    {
      kind: "code",
      payload: { kind: "code", language: "python", source: longCode },
      text: "def pipeline(items, opts): ...",
      meta: { origin: "agent_generated", title: "Transform pipeline" },
    },
  ];

  // Simulate auto-open: the newest pane artifact opens the pane on production.
  onMount(() => {
    const pane = artifacts.find((a) => placement(a) === "pane");
    if (pane) openPane(pane);
  });
</script>

<div class="layout">
  <div class="chat">
    <div class="thread">
      <div class="turn">
        <p class="role">You</p>
        <div class="you">Add helper, chart activity, list two files, and write the pipeline.</div>
      </div>
      <div class="turn">
        <p class="role">Assistant</p>
        <div class="prose">Here you go:</div>
        <div class="arts">
          {#each artifacts as a (a.meta.title)}
            {#if placement(a) === "pane"}
              <ArtifactCard artifact={a} />
            {:else}
              <ArtifactBlock artifact={a} />
            {/if}
          {/each}
        </div>
      </div>
      <div class="turn">
        <p class="role">You</p>
        <div class="you">Looks good. Can you make the chart a line chart instead?</div>
      </div>
      <div class="turn">
        <p class="role">Assistant</p>
        <div class="prose">Sure, switched it to a line. Everything else stays the same.</div>
      </div>
      <div class="turn">
        <p class="role">You</p>
        <div class="you">Perfect, thanks. One more: how do I run the pipeline?</div>
      </div>
      <div class="turn">
        <p class="role">Assistant</p>
        <div class="prose">
          Call it with your list and the options: <code>pipeline(items, opts)</code>. It returns the
          transformed list, leaving the input untouched. The full source is in the pane on the right.
        </div>
      </div>
    </div>
  </div>
  {#if $openArtifact}
    <ArtifactPanel artifact={$openArtifact} onclose={closePane} />
  {/if}
</div>

<style>
  .layout {
    display: flex;
    /* Fill the shell's content area, not the whole viewport: 100vh ignores the
       titlebar and overflows the shell, producing a second (outer) scrollbar. */
    height: 100%;
    min-height: 0;
    background: var(--color-bg-app, var(--background));
  }
  /* The chat area is the scroll region, so its scrollbar sits at the right
     edge of the chat (just left of the pane), not mid-screen. */
  .chat {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .thread {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
    max-width: 46rem;
    padding: 1.5rem 1.5rem 5rem;
    margin: 0 auto;
  }
  .turn {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .role {
    margin: 0;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .you {
    align-self: flex-start;
    padding: 0.5rem 0.75rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    font-size: 0.8125rem;
  }
  .prose {
    font-size: 0.8125rem;
    line-height: 1.55;
    color: var(--foreground);
  }
  .arts {
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
</style>
