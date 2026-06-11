<script lang="ts">
  /// The containment frame every GUI-from-output sits in
  /// (terminal.md §6: output may paint a photograph, never a
  /// switch). The subtle border plus the corner tag say "this came
  /// from the command's output" — rendered chrome stays visually
  /// distinct from the shell's own.
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="output-frame">
  <span class="of-tag">from the output</span>
  <div class="of-content">
    {@render children()}
  </div>
</div>

<style>
  .output-frame {
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 2%, transparent);
    overflow: hidden;
  }

  /* The tag holds its own line so framed content never runs into it. */
  .of-tag {
    display: block;
    text-align: right;
    padding: 4px 12px 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
    pointer-events: none;
  }

  .of-content {
    padding: 4px 12px 12px;
  }
</style>
