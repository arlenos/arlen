<script lang="ts">
  /// Add/remove list of short string chips (app-id allow/suppress lists,
  /// autonomous-apps, tags). Canonical replacement for the bespoke `chips` +
  /// inline add/remove markup. Self-contained text-add by default; bindable
  /// `items` + `onchange`. For path lists prefer `AddRemoveList`; for short
  /// identifiers this compact chip form fits a settings row.
  let {
    items = $bindable([]),
    placeholder = "Add…",
    id,
    disabled = false,
    onchange,
    class: className,
  }: {
    /// The chips (bindable).
    items: string[];
    placeholder?: string;
    /// Optional anchor id for deep-link scroll-to-setting.
    id?: string;
    disabled?: boolean;
    onchange?: (items: string[]) => void;
    class?: string;
  } = $props();

  let draft = $state("");

  function add() {
    const v = draft.trim();
    draft = "";
    if (!v || items.includes(v)) return;
    items = [...items, v];
    onchange?.(items);
  }

  function remove(item: string) {
    items = items.filter((i) => i !== item);
    onchange?.(items);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      add();
    }
  }
</script>

<div class="chiplist {className ?? ''}" {id}>
  {#each items as item (item)}
    <span class="chip">
      <span class="chip-label">{item}</span>
      {#if !disabled}
        <button
          type="button"
          class="chip-x"
          aria-label={`Remove ${item}`}
          onclick={() => remove(item)}>×</button>
      {/if}
    </span>
  {/each}
  {#if !disabled}
    <input
      class="chip-input"
      bind:value={draft}
      {placeholder}
      onkeydown={onkeydown}
      onblur={add} />
  {/if}
</div>

<style>
  .chiplist {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 4px 2px 8px;
    font-size: 0.75rem;
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 14%, transparent);
    border-radius: var(--radius-chip, 4px);
    max-width: 100%;
  }

  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .chip-x {
    appearance: none;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
    font-size: 0.875rem;
    line-height: 1;
    padding: 0 2px;
    border-radius: var(--radius-chip, 4px);
  }
  .chip-x:hover {
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
  }

  .chip-input {
    flex: 1;
    min-width: 6rem;
    appearance: none;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.8125rem;
    padding: 4px 2px;
    outline: none;
  }
  .chip-input::placeholder {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
</style>
