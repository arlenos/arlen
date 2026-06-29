<script lang="ts">
  /// The Save-mode name field. Portal-specific chrome (it has no kit
  /// equivalent), skinned on the kit design tokens so it reads as one
  /// surface with the browser above it. The location is the browsed
  /// folder; the daemon revalidates the composed path.
  import { getUiState, setSaveFilename, validateFilename } from "$lib/stores/pickerUi.svelte";

  let { location }: { location: string } = $props();

  const state = getUiState();

  function onInput(e: Event) {
    setSaveFilename((e.target as HTMLInputElement).value);
  }

  let validationError = $derived(validateFilename(state.saveFilename));
</script>

<div class="save-bar">
  <label class="field">
    <span class="label">Save as</span>
    <input
      type="text"
      placeholder="filename"
      value={state.saveFilename}
      oninput={onInput}
      autocomplete="off"
      spellcheck="false"
      aria-invalid={validationError !== null}
    />
  </label>
  <p class="location" title={location}>in {location}</p>
  {#if validationError && state.saveFilename.length > 0}
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

  input {
    flex: 1;
    height: var(--height-control);
    padding: 0 10px;
    background: var(--color-bg-input);
    color: var(--color-fg-app);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    font-size: 0.875rem;
    outline: none;
    transition: border-color var(--duration-fast) var(--ease-out);
  }

  input:focus {
    border-color: color-mix(in srgb, var(--color-accent) 55%, var(--color-border));
  }

  input[aria-invalid="true"] {
    border-color: var(--color-danger);
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
