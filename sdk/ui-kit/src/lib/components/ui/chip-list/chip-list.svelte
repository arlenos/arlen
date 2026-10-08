<script lang="ts">
  import { kt } from "../../../i18n/messages.kit";
  import { Chip } from "../chip/index.js";
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
    <Chip label={item} onremove={disabled ? undefined : () => remove(item)} />
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
    min-height: var(--height-control, 30px);
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
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
