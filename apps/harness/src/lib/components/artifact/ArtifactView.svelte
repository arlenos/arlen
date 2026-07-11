<script lang="ts">
  /// Renders an artifact payload's body inline, dispatching on its kind. One
  /// branch per inert kind; a malformed payload falls back to the mandatory
  /// plain-text floor so nothing renders blank. Dynamic height: each body is as
  /// tall as its content up to a ceiling, then scrolls in place (no panel, no
  /// "show more"). Text bodies carry `data-selectable` so they can be selected +
  /// copied (the app disables selection globally). No payload grants authority.
  import { t } from "$lib/i18n/messages";
  import { renderMarkdown } from "$lib/markdown";
  import { imageMime, type Artifact } from "./types";
  import ArtifactChart from "./ArtifactChart.svelte";
  import ArtifactCode from "./ArtifactCode.svelte";

  let { artifact }: { artifact: Artifact } = $props();
  const p = $derived(artifact.payload);

  const safeHref = (href: string): string | null =>
    /^(?:https?:\/\/|mailto:)/i.test(href) ? href : null;
</script>

{#if p.kind === "markdown"}
  <div class="markdown scroll" data-selectable>{@html renderMarkdown(p.source)}</div>
{:else if p.kind === "code"}
  <ArtifactCode source={p.source} language={p.language} />
{:else if p.kind === "table"}
  <div class="table-wrap scroll" data-selectable>
    <table>
      <thead>
        <tr>{#each p.columns as c (c)}<th>{c}</th>{/each}</tr>
      </thead>
      <tbody>
        {#each p.rows as row, ri (ri)}
          <tr>{#each row as cell, ci (ci)}<td>{cell}</td>{/each}</tr>
        {/each}
      </tbody>
    </table>
  </div>
{:else if p.kind === "chart"}
  <ArtifactChart chartType={p.chart_type} series={p.series} />
{:else if p.kind === "image"}
  <img class="image" src={`data:${imageMime(p.media_type)};base64,${p.data_base64}`} alt={artifact.meta.title ?? "Image artifact"} />
{:else if p.kind === "diagram"}
  <div class="diagram">
    <span class="diagram-note">Diagram source ({p.language}). Live rendering is coming.</span>
    <pre class="code scroll" data-selectable><code>{p.source}</code></pre>
  </div>
{:else if p.kind === "links"}
  <ul class="links" data-selectable>
    {#each p.links as link (link.href)}
      {@const href = safeHref(link.href)}
      <li>
        {#if href}
          <a {href} rel="noreferrer noopener">{link.label ?? link.href}</a>
        {:else}
          <span class="inert" title={$t("h.artifact.blockedScheme")}>{link.label ?? link.href}</span>
        {/if}
      </li>
    {/each}
  </ul>
{:else}
  <pre class="code scroll" data-selectable><code>{artifact.text}</code></pre>
{/if}

<style>
  /* Dynamic height: as tall as the content up to a ceiling, then scroll in
     place. Code owns its own copy of this in ArtifactCode. */
  .scroll {
    max-height: var(--artifact-max-height, 24rem);
    overflow: auto;
  }

  .markdown {
    font-size: var(--text-sm);
    color: var(--foreground);
    line-height: 1.55;
    cursor: text;
  }

  .code {
    margin: 0;
    padding: 0.75rem 0.875rem;
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    border-radius: var(--radius-input);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--artifact-code-size, 0.75rem);
    line-height: 1.6;
    white-space: pre;
    cursor: text;
    -webkit-user-select: text;
    user-select: text;
  }

  /* Text content selectable with the mouse (the app disables selection
     globally; `text` overrides it, unlike the `data-selectable` -> auto). */
  .markdown,
  .table-wrap {
    -webkit-user-select: text;
    user-select: text;
  }

  .table-wrap {
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    border-radius: var(--radius-input);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-xs);
  }
  th,
  td {
    padding: 0.375rem 0.625rem;
    text-align: left;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
    white-space: nowrap;
  }
  th {
    position: sticky;
    top: 0;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  tbody tr:last-child td {
    border-bottom: none;
  }

  .image {
    max-width: 100%;
    max-height: 24rem;
    height: auto;
    display: block;
    border-radius: var(--radius-input);
  }

  .diagram {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .diagram-note {
    font-size: var(--text-2xs);
    color: var(--color-warning, #d4b483);
  }

  .links {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .links li {
    font-size: var(--text-sm);
  }
  .links a {
    color: var(--color-accent);
    text-decoration: underline;
    overflow-wrap: anywhere;
  }
  .links .inert {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
