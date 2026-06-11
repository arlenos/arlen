<script lang="ts">
  /// The prompt context line above each block's command: the working
  /// directory (home-shortened) and, inside a repository, the branch
  /// with its dirty count — `~/Repositories/arlen/docs | main ?2`
  /// (terminal.md §4.6: full line per block, git inline, colors and
  /// font from the theme).
  import type { GitInfo } from "$lib/contract";
  import { tildify } from "$lib/paths";

  let { cwd, git = null }: { cwd: string; git?: GitInfo | null } = $props();

  const shownPath = $derived(tildify(cwd));
</script>

<span class="prompt-line">
  <span class="pl-path">{shownPath}</span>
  {#if git}
    <span class="pl-sep">|</span>
    <span class="pl-branch">{git.branch}</span>
    {#if git.dirty_count > 0}
      <span class="pl-dirty">?{git.dirty_count}</span>
    {/if}
  {/if}
</span>

<style>
  /* Console content voice, dimmed: same size as the command, the
     hierarchy comes from brightness alone (the command is the only
     full-strength line in a block). */
  .prompt-line {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    min-width: 0;
    max-width: 100%;
    white-space: nowrap;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* Under pressure only the path gives way (ellipsis); branch and
     dirty count stay whole — a clipped branch name is worse than a
     shortened path. */
  .pl-path {
    flex-shrink: 1;
    min-width: 4ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pl-sep,
  .pl-branch,
  .pl-dirty {
    flex-shrink: 0;
  }

  .pl-sep {
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }

  .pl-dirty {
    color: var(--color-warning);
  }
</style>
