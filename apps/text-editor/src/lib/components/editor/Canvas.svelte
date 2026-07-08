<script lang="ts">
  /// The text canvas, in the iA-Writer stance: markdown syntax stays VISIBLE but is
  /// styled - you always see the real bytes you own, never a WYSIWYG that hides them.
  /// Prose is line-number-free (iA); fenced code blocks get a line-number gutter +
  /// tonal syntax highlighting (monochrome, no rainbow). Focus mode fades every prose
  /// paragraph but the active one. This is the surface; the incremental tree-sitter/
  /// LSP engine that highlights + numbers real files is the coder's, so the fixture
  /// renders read-mostly here.
  let {
    doc,
    focusMode = false,
    fileType = "markdown",
    lineNumbers = true,
  }: {
    doc: string;
    focusMode?: boolean;
    fileType?: "markdown" | "code";
    lineNumbers?: boolean;
  } = $props();

  let activeIdx = $state(1);

  // A code/text file is one whole highlighted document (no markdown parsing, no
  // focus fade); its line numbers are the whole-file gutter.
  const codeLines = $derived(doc.replace(/\n$/, "").split("\n"));

  type Segment =
    | { kind: "prose"; blocks: string[] }
    | { kind: "code"; lang: string; lines: string[] };

  // Split the doc into ordered prose / fenced-code segments.
  const segments = $derived.by<Segment[]>(() => {
    const out: Segment[] = [];
    const fence = /```(\w*)\n([\s\S]*?)```/g;
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = fence.exec(doc)) !== null) {
      const before = doc.slice(last, m.index).trim();
      if (before) out.push({ kind: "prose", blocks: before.split(/\n\n+/) });
      out.push({ kind: "code", lang: m[1] || "text", lines: m[2].replace(/\n$/, "").split("\n") });
      last = fence.lastIndex;
    }
    const rest = doc.slice(last).trim();
    if (rest) out.push({ kind: "prose", blocks: rest.split(/\n\n+/) });
    return out;
  });

  // A running index over PROSE blocks only, so focus mode tracks the active paragraph.
  const proseIndex = $derived.by(() => {
    const map: number[][] = [];
    let bi = 0;
    for (const seg of segments) {
      if (seg.kind === "prose") map.push(seg.blocks.map(() => bi++));
      else map.push([]);
    }
    return map;
  });

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
      return `<span class="md-mark">${marks} </span>${inline(block.slice(level + 1))}`;
    }
    return inline(block.replace(/\n/g, " "));
  }

  // A light, single-pass tonal highlighter (the surface; real tokenisation is
  // tree-sitter, a coder seam). Monochrome tones, never a rainbow.
  const KW = /^(const|let|var|function|return|if|else|for|while|async|await|import|from|export|default|type|interface|enum|new|class|extends|fn|pub|use|struct|impl|match|Some|None|true|false|null|void|string|number|boolean)$/;
  function highlight(line: string): string {
    const e = esc(line);
    const re = /(\/\/[^\n]*)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`(?:[^`\\]|\\.)*`)|(\b\d[\w.]*\b)|([A-Za-z_$][\w$]*)/g;
    return e.replace(re, (mm, comment, str, num, word) => {
      if (comment) return `<span class="tok-comment">${comment}</span>`;
      if (str) return `<span class="tok-str">${str}</span>`;
      if (num) return `<span class="tok-num">${num}</span>`;
      if (word) return KW.test(word) ? `<span class="tok-kw">${word}</span>` : word;
      return mm;
    });
  }
</script>

{#if fileType === "code"}
  <div class="codefile" class:numbers={lineNumbers}>
    {#if lineNumbers}
      <div class="cf-gutter" aria-hidden="true">
        {#each codeLines as _, i (i)}
          <span class="cf-ln">{i + 1}</span>
        {/each}
      </div>
    {/if}
    <pre class="cf-code"><code>{#each codeLines as line, i (i)}<span class="cf-line">{@html highlight(line) || "&nbsp;"}</span>{#if i < codeLines.length - 1}{"\n"}{/if}{/each}</code></pre>
  </div>
{:else}
<div class="canvas" class:focus={focusMode}>
  {#each segments as seg, si (si)}
    {#if seg.kind === "prose"}
      {#each seg.blocks as block, bi (bi)}
        {@const idx = proseIndex[si][bi]}
        {@const level = headingLevel(block)}
        <p
          class="blk"
          class:active={idx === activeIdx}
          class:h1={level === 1}
          class:h2={level === 2}
          class:h3={level >= 3}
          role="presentation"
          onclick={() => (activeIdx = idx)}
        >
          {@html renderBlock(block)}
        </p>
      {/each}
    {:else}
      <div class="code-block">
        <span class="code-lang">{seg.lang}</span>
        <div class="code-body">
          <div class="gutter" aria-hidden="true">
            {#each seg.lines as _, i (i)}
              <span class="ln">{i + 1}</span>
            {/each}
          </div>
          <pre class="code"><code>{#each seg.lines as line, i (i)}<span class="code-line">{@html highlight(line) || "&nbsp;"}</span>{#if i < seg.lines.length - 1}{"\n"}{/if}{/each}</code></pre>
        </div>
      </div>
    {/if}
  {/each}
</div>
{/if}

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
  .canvas.focus .blk {
    opacity: 0.28;
  }
  .canvas.focus .blk.active {
    opacity: 1;
  }

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

  /* Fenced code: a line-number gutter + tonal (monochrome) highlighting. */
  .code-block {
    position: relative;
    margin: 0 0 1.5rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
    overflow: hidden;
  }
  .code-lang {
    position: absolute;
    top: 0.5rem;
    right: 0.75rem;
    font-size: 0.6875rem;
    letter-spacing: 0.03em;
    color: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
  }
  .code-body {
    display: flex;
    font-size: 0.8125rem;
    line-height: 1.6;
  }
  .gutter {
    display: flex;
    flex-direction: column;
    padding: 0.85rem 0.5rem 0.85rem 0.75rem;
    text-align: right;
    user-select: none;
    border-right: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .ln {
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 28%, transparent);
  }
  .code {
    margin: 0;
    padding: 0.85rem 1rem;
    overflow-x: auto;
    color: color-mix(in srgb, var(--color-fg-primary) 82%, transparent);
  }
  :global(.tok-kw) {
    color: var(--color-fg-primary);
    font-weight: 600;
  }
  :global(.tok-str) {
    color: color-mix(in srgb, var(--color-fg-primary) 62%, transparent);
  }
  :global(.tok-comment) {
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
    font-style: italic;
  }
  :global(.tok-num) {
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }

  /* Code/text file: the whole document as one highlighted view with an optional
     whole-file line-number gutter. */
  .codefile {
    display: flex;
    font-family: var(--font-mono, ui-monospace, "SF Mono", monospace);
    font-size: 0.875rem;
    line-height: 1.65;
  }
  .cf-gutter {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    padding: 0 0.9rem 0 0.25rem;
    text-align: right;
    user-select: none;
  }
  .cf-ln {
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 26%, transparent);
  }
  .cf-code {
    margin: 0;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
    color: color-mix(in srgb, var(--color-fg-primary) 82%, transparent);
  }
</style>
