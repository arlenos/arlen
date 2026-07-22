<script lang="ts">
  /// The editor window: a two-pane surface - the text canvas + the KG-lens panel.
  /// The lens is a co-star (it is the reason the editor exists), not a hidden
  /// sidebar. The slim titlebar carries the file name, a focus-mode toggle, and the
  /// as-of scrubber (time-travel over the file + its context).
  import Canvas from "$lib/components/editor/Canvas.svelte";
  import LensPanel from "$lib/components/editor/LensPanel.svelte";
  import AiEditReview from "$lib/components/editor/AiEditReview.svelte";
  import { loadLens } from "$lib/stores/lens";
  import { proposal, proposeEdit, dismiss } from "$lib/stores/aiEdit";
  import { t, dir } from "$lib/i18n/messages";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Sun, PanelRight, Hash } from "lucide-svelte";

  // The AI edit is invoked by keyboard (Cmd/Ctrl+K), never a bolted-on titlebar
  // button. Its discoverable home is a future command palette; a text-selection
  // "edit this" action is the contextual one. Escape dismisses an open proposal.
  function onKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      if (!$proposal) void proposeEdit("Tighten the intro and add a reference");
    } else if (e.key === "Escape" && $proposal) {
      e.preventDefault();
      dismiss();
    }
  }

  // The transaction-time presets (mirrors apps/files/src/lib/asof.ts). Derived so the
  // labels re-resolve when the locale switches.
  const AS_OF_OPTIONS = $derived([
    { value: "now", label: $t("te.asOf.now") },
    { value: "1d", label: $t("te.asOf.1d") },
    { value: "1w", label: $t("te.asOf.1w") },
    { value: "1m", label: $t("te.asOf.1m") },
  ]);

  let focusMode = $state(false);
  let lensOpen = $state(true);
  let asOf = $state("now");
  let fileIdx = $state(0);
  let lineNumbers = $state(true);

  const MD_DOC = `# The KG-lens

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

  const CODE_DOC = `// The Arlen gate: every AI tool call is authorized before it runs.
import { invoke } from "@tauri-apps/api/core";

export type AuthorizeDecision =
  | { decision: "allow"; proof?: string }
  | { decision: "confirm"; prompt: string }
  | { decision: "deny"; reason: string };

// Reversible edits run autonomously; irreversible ones are held for the user.
export async function authorize(call: ToolCall): Promise<AuthorizeDecision> {
  const verdict = await invoke("Authorize", { call });
  if (verdict.decision === "deny") {
    return { decision: "deny", reason: verdict.reason };
  }
  return verdict;
}`;

  const FILES = [
    { name: "the-kg-lens.md", type: "markdown" as const, content: MD_DOC },
    { name: "gate.ts", type: "code" as const, content: CODE_DOC },
  ];
  const file = $derived(FILES[fileIdx]);
  const fileOptions = FILES.map((f, i) => ({ value: String(i), label: f.name }));

  // The lens tracks whichever file is open.
  $effect(() => {
    loadLens(file.name);
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app" dir={$dir}>
  <header class="titlebar">
    <PopoverSelect
      value={String(fileIdx)}
      options={fileOptions}
      width="170px"
      ariaLabel={$t("te.openFile")}
      onchange={(v) => (fileIdx = Number(v))}
    />
    <span class="spacer"></span>
    {#if file.type === "code"}
      <button
        type="button"
        class="tb-btn icon"
        class:on={lineNumbers}
        aria-label={$t("te.lineNumbers.toggle")}
        title={$t("te.lineNumbers")}
        onclick={() => (lineNumbers = !lineNumbers)}
      >
        <Hash size={15} strokeWidth={2} />
      </button>
    {:else}
      <button type="button" class="tb-btn" class:on={focusMode} onclick={() => (focusMode = !focusMode)}>
        <Sun size={14} strokeWidth={2} /> {$t("te.focus")}
      </button>
    {/if}
    <PopoverSelect
      value={asOf}
      options={AS_OF_OPTIONS}
      width="130px"
      ariaLabel={$t("te.asOf.aria")}
      onchange={(v) => (asOf = v)}
    />
    <button
      type="button"
      class="tb-btn icon"
      class:on={lensOpen}
      aria-label={$t("te.lens.toggle")}
      title={$t("te.lens.toggle")}
      onclick={() => (lensOpen = !lensOpen)}
    >
      <PanelRight size={15} strokeWidth={2} />
    </button>
  </header>

  <div class="body">
    <main class="editor">
      <Canvas doc={file.content} fileType={file.type} {focusMode} {lineNumbers} />
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
    font-size: var(--text-sm);
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
