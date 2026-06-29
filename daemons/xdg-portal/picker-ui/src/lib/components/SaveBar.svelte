<script lang="ts">
  /// The Save-mode name field, on the kit `Input` so it matches every
  /// other field in the system. The location is the browsed folder; the
  /// daemon revalidates the composed path.
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { getUiState, setSaveFilename, validateFilename } from "$lib/stores/pickerUi.svelte";

  let { location }: { location: string } = $props();

  const ui = getUiState();
  let filename = $state(ui.saveFilename);

  // Keep the local field in sync when a request seeds the name (the
  // caller's currentName / a file activation reusing its name).
  $effect(() => {
    filename = ui.saveFilename;
  });

  let validationError = $derived(validateFilename(ui.saveFilename));
</script>

<div class="save-bar">
  <label class="field">
    <span class="label">Save as</span>
    <Input
      placeholder="filename"
      bind:value={filename}
      oninput={() => setSaveFilename(filename)}
      autocomplete="off"
      spellcheck="false"
      aria-invalid={validationError !== null}
    />
  </label>
  <p class="location" title={location}>in {location}</p>
  {#if validationError && ui.saveFilename.length > 0}
    <p class="error">{validationError}</p>
  {/if}
</div>

<style>
  .save-bar {
    padding: 10px 16px;
    border-top: 1px solid var(--color-border);
    background: var(--color-bg-card);
  }

  .field {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .label {
    flex-shrink: 0;
    font-size: 0.8125rem;
    color: var(--color-fg-muted);
  }

  .location {
    margin: 6px 0 0;
    padding-left: calc(0.8125rem + 10px);
    font-size: 0.75rem;
    color: var(--color-fg-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .error {
    margin: 4px 0 0;
    font-size: 0.75rem;
    color: var(--color-danger);
  }
</style>
