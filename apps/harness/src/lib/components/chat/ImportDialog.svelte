<script lang="ts">
  /// The import-chat dialog: a small modal (like the share-context flow) to bring in a
  /// conversation exported as JSON. A clickable drop zone picks or accepts a file; the
  /// store validates + adds it as a new conversation. A bad file shows the error inside
  /// the dialog, so nothing leaks into the sidebar. Mounted once in the layout.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import Dialog from "@arlen/ui-kit/components/ui/dialog/dialog.svelte";
  import { Upload } from "@lucide/svelte";
  import { importOpen, closeImportChat } from "$lib/stores/importChat";
  import { importConversation } from "$lib/stores/conversation";

  let fileInput = $state<HTMLInputElement | null>(null);
  let error = $state("");
  let dragging = $state(false);

  // A fresh open starts clean.
  $effect(() => {
    if ($importOpen) error = "";
  });

  async function handleFile(file: File | undefined): Promise<void> {
    if (!file) return;
    const id = importConversation(await file.text());
    if (id === null) {
      error = "That file is not a chat export.";
      return;
    }
    closeImportChat();
    if ($page.url.pathname.startsWith("/agent")) goto("/");
  }
  function onPick(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    handleFile(file);
  }
  function onDrop(e: DragEvent): void {
    e.preventDefault();
    dragging = false;
    handleFile(e.dataTransfer?.files?.[0]);
  }
</script>

<Dialog open={$importOpen} onClose={closeImportChat} size="sm" ariaLabel="Import chat">
  <div class="imp">
    <header class="imp-head">
      <h2 class="imp-title">Import chat</h2>
      <p class="imp-lead">
        Open a chat you exported as JSON. It is added as a new conversation, so it never overwrites one you have.
      </p>
    </header>

    <button
      type="button"
      class="drop"
      class:dragging
      onclick={() => fileInput?.click()}
      ondragover={(e) => {
        e.preventDefault();
        dragging = true;
      }}
      ondragleave={() => (dragging = false)}
      ondrop={onDrop}
    >
      <Upload size={20} strokeWidth={1.75} />
      <span class="drop-label">Drop a chat file here, or choose one</span>
      <span class="drop-hint">A .json file exported from a chat</span>
    </button>

    {#if error}
      <p class="imp-error" role="alert">{error}</p>
    {/if}

    <input
      bind:this={fileInput}
      type="file"
      accept="application/json,.json"
      class="sr-only"
      tabindex="-1"
      aria-hidden="true"
      onchange={onPick}
    />
  </div>
</Dialog>

<style>
  .imp {
    padding: 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  .imp-head {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .imp-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
  }
  .imp-lead {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.4rem;
    padding: 1.75rem 1rem;
    border: 1px dashed color-mix(in srgb, var(--foreground) 25%, transparent);
    border-radius: var(--radius-card, 12px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    transition:
      border-color var(--duration-fast) var(--ease-out),
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .drop:hover,
  .drop.dragging {
    border-color: color-mix(in srgb, var(--foreground) 45%, transparent);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    color: var(--foreground);
  }
  .drop-label {
    font-size: 0.875rem;
  }
  .drop-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .imp-error {
    margin: 0;
    font-size: 0.8125rem;
    color: var(--destructive, #c96a6a);
  }
</style>
