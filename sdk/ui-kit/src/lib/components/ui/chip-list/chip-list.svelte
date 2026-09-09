<script lang="ts">
  import { kt } from "../../../i18n/messages.kit";
  /// Add/remove list of short string chips (app-id allow/suppress lists,
  /// autonomous-apps, tags). Canonical replacement for the bespoke `chips` +
  /// inline add/remove markup. Self-contained text-add by default; bindable
  /// `items` + `onchange`. For path lists prefer `AddRemoveList`; for short
  /// identifiers this compact chip form fits a settings row.
  let {
    items = $bindable([]),
    placeholder,
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
    <!-- Commit only on Enter (explicit), never on blur: a half-typed or invalid
         value must not be silently persisted by tabbing/clicking away, since
         consumers persist on `onchange` (e.g. an app-id allow list). The draft
         is discarded on blur. -->
    <input
      class="chip-input"
      bind:value={draft}
      placeholder={placeholder ?? $kt("k.chip.add")}
      onkeydown={onkeydown} />
  {/if}
</div>

<style>
  .chiplist {
    min-height: var(--height-control, 28px);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: var(--height-control-compact, 24px);
    padding: 0 4px 0 8px;
    font-size: var(--text-xs);
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
    font-size: var(--text-base);
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
    font-size: var(--text-sm);
    padding: 4px 2px;
    outline: none;
  }
  /* Keyboard focus has to land somewhere a person can see. The input suppresses
     the browser's outline like every hand-styled control in the kit, and unlike
     the number input and the slider it had nothing in its place - so tabbing
     into a chip list put the caret in a borderless transparent box on a
     borderless transparent row, with no signal at all. Found by the render
     sweep on 9 September, on the Knowledge and AI pages (three inputs and one),
     which are the same component seen twice.

     An underline rather than the wrapper ring those two use: their wrapper IS
     the visible control, a field box, and this one is a bare row of chips whose
     input is a small part of it. Ringing the whole row would point at the chips
     rather than at the place the next keystroke goes. */
  .chip-input:focus-visible {
    box-shadow: inset 0 -2px 0 0 var(--color-accent, var(--primary));
  }

  .chip-input::placeholder {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  /* The remove button had hover styling and no focus styling, so it was
     reachable by keyboard and invisible while reached. Same accent, and the
     hover background too, because the button is a small glyph and a ring alone
     on it reads as a rendering artefact. */
  .chip-x:focus-visible {
    outline: 2px solid var(--color-accent, var(--primary));
    outline-offset: 1px;
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
  }
</style>
