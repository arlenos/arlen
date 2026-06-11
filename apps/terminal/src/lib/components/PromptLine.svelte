<script lang="ts">
  /// The prompt context line above each block's command: the working
  /// directory (home-shortened) and, inside a repository, the branch
  /// with its dirty count — `~/Repositories/arlen/docs | main ?2`
  /// (terminal.md §4.6: full line per block, git inline, colors and
  /// font from the theme).
  import type { GitInfo } from "$lib/contract";

  let { cwd, git = null }: { cwd: string; git?: GitInfo | null } = $props();

  /// `/home/<user>` becomes `~` for display. The contract ships
  /// absolute paths; this is presentation only.
  const shownPath = $derived.by(() => {
    const m = cwd.match(/^\/home\/[^/]+(\/.*)?$/);
    if (!m) return cwd;
    return "~" + (m[1] ?? "");
  });
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
  .prompt-line {
    display: inline-flex;
    align-items: baseline;
    gap: 6px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.6875rem;
    line-height: 1.5;
    min-width: 0;
  }

  .pl-path {
    color: color-mix(in srgb, var(--color-accent, var(--primary)) 75%, var(--foreground) 25%);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pl-sep {
    color: color-mix(in srgb, var(--foreground) 30%, transparent);
  }

  .pl-branch {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .pl-dirty {
    color: var(--color-warning, #eab308);
  }
</style>
