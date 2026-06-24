<script lang="ts">
  /// The prompt context line above each block's command: the working
  /// directory (home-shortened) and, inside a repository, the branch
  /// with its dirty count — `~/Repositories/arlen/docs | main ?2`
  /// (terminal.md §4.6: full line per block, git inline, colors and
  /// font from the theme).
  import type { GitInfo } from "$lib/contract";
  import { collapsePath } from "$lib/paths";

  let { cwd, git = null }: { cwd: string; git?: GitInfo | null } = $props();

  const path = $derived(collapsePath(cwd));
</script>

<span class="prompt-line">
  <span class="pl-path"><span class="pl-prefix">{path.prefix}</span><span class="pl-anchor">{path.anchor}</span></span>
  {#if git}
    <span class="pl-branch">{git.branch}</span>
    {#if git.dirty_count > 0}
      <span class="pl-dirty">*</span>
    {/if}
  {/if}
</span>

<style>
  .prompt-line {
    display: inline-flex;
    align-items: baseline;
    gap: 8px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--console-font-size, 0.8125rem);
    line-height: 1.5;
    min-width: 0;
    max-width: 100%;
    white-space: nowrap;
  }

  /* The path reads p10k-style: the trail to the current folder is faint
     (abbreviated ancestors), the current folder is the solid anchor. Branch
     in the muted green of the palette. The colours reference the ANSI palette
     vars so they follow the theme once it projects them, with the muted
     defaults until then. */
  .pl-path {
    flex-shrink: 1;
    min-width: 2ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pl-prefix {
    color: color-mix(in srgb, var(--term-ansi-4, #7d9cc4) 55%, transparent);
  }
  .pl-anchor {
    color: var(--term-ansi-4, #7d9cc4);
    font-weight: 600;
  }
  .pl-branch {
    color: var(--term-ansi-2, #8fae74);
    flex-shrink: 0;
  }
  .pl-dirty {
    color: var(--term-ansi-3, #d4b483);
    flex-shrink: 0;
    margin-left: -4px;
  }
</style>
