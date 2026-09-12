<script lang="ts">
  /// A single row inside a Group card. Label on the left, control on the
  /// right, with an optional inline preview between them, and an optional
  /// full-width `below` area for wide controls (a list, a chip editor) that
  /// do not fit the right-aligned control slot.
  ///
  /// A row can be overridden: its value was set on top of something that
  /// would otherwise decide it (a theme, a default). Then an accent bar sits
  /// in the left gutter (the modified mark) and a reset control appears
  /// before the control to fall back. The control always shows the resolved
  /// value at full contrast; there is never a greyed placeholder for what the
  /// fallback would be.
  import type { Snippet } from "svelte";
  import { RotateCcw } from "@lucide/svelte";
  import { IconAction } from "../icon-action";
  import { kt } from "../../../i18n/messages.kit";

  let {
    label,
    id: rowId,
    description,
    overridden = false,
    onreset,
    leading,
    control,
    preview,
    below,
  }: {
    label: string;
    /// Optional anchor id for deep-link scroll-to-setting.
    id?: string;
    description?: string;
    /// True when the value is set on top of a fallback (shows the bar and,
    /// with `onreset`, the reset control).
    overridden?: boolean;
    /// Fall back to the value the override replaced.
    onreset?: () => void;
    /// Optional leading visual before the label (a logo, badge, or rank), for
    /// rows that need an icon column the bare label/control layout lacks.
    leading?: Snippet;
    control?: Snippet;
    preview?: Snippet;
    /// Optional full-width content rendered under the label/control line.
    below?: Snippet;
  } = $props();
</script>

<div class="row" class:overridden id={rowId}>
  <div class="row-main">
    {#if leading}
      <div class="leading">{@render leading()}</div>
    {/if}
    <div class="label">
      <div class="label-title">{label}</div>
      {#if description}
        <div class="label-desc">{description}</div>
      {/if}
    </div>
    {#if preview}
      <div class="preview">
        {@render preview()}
      </div>
    {/if}
    <div class="control">
      {#if overridden && onreset}
        <span class="reset">
          <IconAction label={$kt("k.row.reset", { name: label })} size="compact" onclick={onreset}>
            <RotateCcw size={13} strokeWidth={2} />
          </IconAction>
        </span>
      {/if}
      {@render control?.()}
    </div>
  </div>
  {#if below}
    <div class="row-below">
      {@render below()}
    </div>
  {/if}
</div>

<style>
  .row {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    /* Top/bottom uses the row spacing token; horizontal stays fixed at 1rem
       to give the label a reliable left edge regardless of token overrides. */
    padding: var(--space-row, 0.75rem) 1rem;
  }

  /* The modified mark: a bar in the left gutter, the accent at rest. */
  .row.overridden::before {
    content: "";
    position: absolute;
    inset-block: 0.5rem;
    inset-inline-start: 0;
    width: 2px;
    border-radius: var(--radius-full);
    background: var(--color-accent, var(--foreground));
  }

  .row-main {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    /* Preserve the row rhythm: the main line keeps the standard row
       height; the .row padding is excluded here so a `below` block adds
       under it rather than inflating the line. */
    min-height: calc(var(--height-row, 40px) - 1.5rem);
  }

  .leading {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }

  .label {
    /* `1 1 auto`, not `1`. `flex: 1` is basis ZERO, so the label never took part
       in shrinking - it only grew into whatever the inflexible items left, and a
       wide preview beside it left almost nothing. With an auto basis both sides
       start from their content width and give up pixels in proportion, so the
       bigger one loses more. */
    flex: 1 1 auto;
    min-width: 0;
  }

  /* Title truncates; description wraps to multiple lines if it
     can't fit on one. This keeps the row height stable per
     row-rhythm spec while still allowing prose-style hints
     below the title. */
  .label-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .label-desc {
    font-size: var(--text-2xs);
    line-height: 1.3;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    margin-top: 0.0625rem;
  }

  /* The control may give ground, but never below its own minimum. `flex-shrink: 0`
     meant a wide control took its full width and the label absorbed every pixel of
     the shortfall: on the sound page at 720px in German a 272px picker left the
     label 86 where it needed 118, and "Gerät angeschlossen" ellipsed to "Gerät
     ang...". `min-content` is the floor that makes this safe for the fixed-size
     controls - a switch cannot be squashed below a switch - while a picker or a
     button pair gives back the slack it was holding. */
  .control {
    flex-shrink: 1;
    min-width: min-content;
  }

  /* THE PREVIEW IS CAPPED SO THE NAME SURVIVES. It used to sit at
     `flex-shrink: 0` beside the control, and the label is `flex: 1` - basis 0,
     so it does not shrink, it GROWS into whatever the inflexible items leave.
     A wide preview therefore took its content width first and the label got the
     remainder. Measured at 720px in German on the AI providers page: "GitHub
     Copilot" was allotted 20px while the badges beside it kept 155 and the
     control 132. The name of the thing a row is about is the last thing a person
     can afford to lose.

     `flex-shrink: 1` alone does NOT fix it - measured, same 155px - because the
     line is not overflowing, so nothing shrinks; the label is simply last in the
     queue. A proportion is what actually bounds it, and a proportion rather than
     a pixel floor because it holds at every width the sweep renders. The control
     keeps `flex-shrink: 0`: it is an operable thing with a hit target, not a
     description.

     NO `overflow: hidden` HERE, and it was there for one run. Clipping the box is
     what turns a squeeze into a CUT: the sweep came back with "Inoffiziell cut
     76px sideways by .preview", a badge sliced mid-word, which is the trade the
     label was just rescued from. A preview that cannot fit should wrap or spill
     where its own content decides, and the content is the caller's - a chip row
     that can wrap says so itself. */
  .preview {
    flex-shrink: 1;
    min-width: 0;
  }

  /* The reset sits before the control and only shows itself when the row is
     pointed at or focused within, so a page of overrides does not become a
     page of undo buttons. */
  .control {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
  }
  .reset {
    display: inline-flex;
    opacity: 0;
    transition: opacity var(--duration-fast, 120ms) var(--ease-default, ease);
  }
  .row:hover .reset,
  .row:focus-within .reset {
    opacity: 1;
  }

  /* The row-control register: every box-shaped field inside a Row's control
     slot (NumberInput, PopoverSelect, text fields that opt in) picks up one
     shared width, so the left edges of the control column align down the whole
     page. Intrinsic controls (Switch, Checkbox, buttons) ignore it. Pages tune
     the register through the --width-row-control token, never per control; an
     explicit width prop on a control still wins for the rare exception. */
  .control {
    --control-width: var(--width-row-control, 200px);
  }
  /* Bare text fields join the register too (the kit Input is w-full, which
     collapses to intrinsic inside the shrink-to-fit control slot). Scoped to
     the Input's data-slot so composed controls' inner inputs (NumberInput,
     ChipList) stay untouched; a flexing wrapper (flex-basis) still wins for
     composed clusters like a path field with its browse button. */
  .control :global(input[data-slot="input"]) {
    width: var(--control-width, auto);
  }

  /* Full-width content under the row line. */
  .row-below {
    min-width: 0;
  }
</style>
