<script lang="ts">
  /// The editor window: a two-pane surface - the text canvas + the KG-lens panel.
  /// The lens is a co-star (it is the reason the editor exists), not a hidden
  /// sidebar. The slim titlebar carries the file name, a focus-mode toggle, and the
  /// as-of scrubber (time-travel over the file + its context).
  import { onMount } from "svelte";
  import Canvas from "$lib/components/editor/Canvas.svelte";
  import LensPanel from "$lib/components/editor/LensPanel.svelte";
  import AiEditReview from "$lib/components/editor/AiEditReview.svelte";
  import { loadLens } from "$lib/stores/lens";
  import { proposal, proposeEdit } from "$lib/stores/aiEdit";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Sun, PanelRight, Sparkles } from "lucide-svelte";

  // The transaction-time presets (mirrors apps/files/src/lib/asof.ts).
  const AS_OF_OPTIONS = [
    { value: "now", label: "Now" },
    { value: "1d", label: "1 day ago" },
    { value: "1w", label: "1 week ago" },
    { value: "1m", label: "1 month ago" },
  ];

  let focusMode = $state(false);
  let lensOpen = $state(true);
  let asOf = $state("now");

  const DOC = `# The KG-lens

This file is a **first-class citizen** of the knowledge graph. Beside the text, Arlen surfaces where it came from, the notes that mention it, and the project it belongs to.

## Why not gedit

A plain editor is a solved category. The reason to build our own is the lens and the [gated AI-edit](lens-design.md): the assistant is a bounded, auditable, reversible principal that can edit this file.

## The gate, in code

Before the assistant writes, its edit is authorized:

\`\`\`ts
type AuthorizeDecision =
  | { decision: "allow" }                     // reversible, autonomous
  | { decision: "confirm"; prompt: string }   // irreversible, ask first
  | { decision: "deny"; reason: string };
\`\`\`

## Focus mode

Turn this on and every paragraph but the one you are in fades away, so the writing is all that is left. The markdown you see is the real \`bytes\` on disk, never hidden.`;

  onMount(() => loadLens("the-kg-lens.md"));
</script>

<div class="app">
  <header class="titlebar">
    <span class="file">the-kg-lens.md</span>
    <span class="spacer"></span>
    <button type="button" class="tb-btn" onclick={() => proposeEdit("Tighten the intro and add a reference")}>
      <Sparkles size={14} strokeWidth={2} /> Ask the assistant
    </button>
    <button type="button" class="tb-btn" class:on={focusMode} onclick={() => (focusMode = !focusMode)}>
      <Sun size={14} strokeWidth={2} /> Focus
    </button>
    <PopoverSelect
      value={asOf}
      options={AS_OF_OPTIONS}
      width="130px"
      ariaLabel="Show the file as of"
      onchange={(v) => (asOf = v)}
    />
    <button
      type="button"
      class="tb-btn icon"
      class:on={lensOpen}
      aria-label="Toggle the lens"
      title="Toggle the lens"
      onclick={() => (lensOpen = !lensOpen)}
    >
      <PanelRight size={15} strokeWidth={2} />
    </button>
  </header>

  <div class="body">
    <main class="editor">
      <Canvas doc={DOC} {focusMode} />
    </main>
    {#if $proposal}
      <AiEditReview />
    {:else if lensOpen}
      <LensPanel />
    {/if}
  </div>
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  .titlebar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    height: 2.75rem;
    padding: 0 1rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    flex-shrink: 0;
  }
  .file {
    font-size: 0.8125rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .spacer {
    flex: 1;
  }
  .tb-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    cursor: pointer;
  }
  .tb-btn:hover {
    color: var(--color-fg-primary);
  }
  .tb-btn.on {
    border-color: color-mix(in srgb, var(--color-fg-primary) 30%, transparent);
    color: var(--color-fg-primary);
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .tb-btn.icon {
    padding: 0.3rem;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .editor {
    flex: 1;
    overflow-y: auto;
    padding: 1.5rem 2rem;
  }
</style>
