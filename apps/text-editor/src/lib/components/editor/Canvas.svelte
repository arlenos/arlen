<script lang="ts">
  /// The text canvas, in the iA-Writer stance: markdown syntax stays VISIBLE but is
  /// styled - you always see the real bytes you own (sovereignty-aligned), never a
  /// WYSIWYG that hides them. Focus mode fades every paragraph but the active one.
  /// This is the surface; the incremental tree-sitter/LSP editing engine is the
  /// coder's, so the fixture doc renders read-mostly here.
  let { doc, focusMode = false }: { doc: string; focusMode?: boolean } = $props();

  let activeIdx = $state(1);

  const blocks = $derived(doc.trim().split(/\n\n+/));

  function esc(s: string): string {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }
  // Style inline markers while KEEPING the marker characters visible (the iA stance).
  function inline(s: string): string {
    let t = esc(s);
    t = t.replace(
      /\*\*([^*]+)\*\*/g,
      '<span class="md-mark">**</span><span class="md-strong">$1</span><span class="md-mark">**</span>',
    );
    t = t.replace(
      /`([^`]+)`/g,
      '<span class="md-mark">`</span><span class="md-code">$1</span><span class="md-mark">`</span>',
    );
    t = t.replace(
      /\[([^\]]+)\]\(([^)]+)\)/g,
      '<span class="md-mark">[</span><span class="md-link">$1</span><span class="md-mark">](</span><span class="md-url">$2</span><span class="md-mark">)</span>',
    );
    return t;
  }
  function headingLevel(block: string): number {
    const m = block.match(/^(#{1,6}) /);
    return m ? m[1].length : 0;
  }
  function renderBlock(block: string): string {
    const level = headingLevel(block);
    if (level > 0) {
      const marks = "#".repeat(level);
      const rest = block.slice(level + 1);
      return `<span class="md-mark">${marks} </span>${inline(rest)}`;
    }
    return inline(block.replace(/\n/g, " "));
  }
</script>

<div class="canvas" class:focus={focusMode}>
  {#each blocks as block, i (i)}
    {@const level = headingLevel(block)}
    <p
      class="blk"
      class:active={i === activeIdx}
      class:h1={level === 1}
      class:h2={level === 2}
      class:h3={level >= 3}
      role="presentation"
      onclick={() => (activeIdx = i)}
    >
      {@html renderBlock(block)}
    </p>
  {/each}
</div>

<style>
  .canvas {
    max-width: 40rem;
    margin: 0 auto;
    padding: 1rem 0 6rem;
    font-family: var(--font-mono, ui-monospace, "SF Mono", monospace);
    font-size: 1rem;
    line-height: 1.85;
    color: color-mix(in srgb, var(--color-fg-primary) 88%, transparent);
  }
  .blk {
    margin: 0 0 1.35rem;
    cursor: text;
    transition: opacity var(--duration-micro, 120ms) var(--ease-out, ease);
  }
  .blk.h1 {
    font-size: 1.55rem;
    font-weight: 600;
    line-height: 1.3;
    color: var(--color-fg-primary);
  }
  .blk.h2 {
    font-size: 1.2rem;
    font-weight: 600;
    line-height: 1.35;
    color: var(--color-fg-primary);
  }
  .blk.h3 {
    font-size: 1.05rem;
    font-weight: 600;
    color: var(--color-fg-primary);
  }

  /* Focus mode: only the active paragraph stays lit. */
  .canvas.focus .blk {
    opacity: 0.28;
  }
  .canvas.focus .blk.active {
    opacity: 1;
  }

  /* iA stance: the markers stay visible, just quietened; the content is styled. */
  :global(.md-mark) {
    color: color-mix(in srgb, var(--color-fg-primary) 28%, transparent);
  }
  :global(.md-strong) {
    font-weight: 700;
    color: var(--color-fg-primary);
  }
  :global(.md-code) {
    color: var(--color-fg-primary);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    padding: 0 0.2em;
    border-radius: var(--radius-chip, 4px);
  }
  :global(.md-link) {
    color: var(--color-fg-primary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }
  :global(.md-url) {
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
