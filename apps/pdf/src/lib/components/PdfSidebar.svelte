<script lang="ts">
  /// The reader's rail: search on top, then EITHER the hits OR the contents,
  /// never both - a search in progress replaces the outline the way it does in
  /// every reader. The honesty rows travel with it: pages nobody could read
  /// are counted beside the hits, an unresolvable heading stays visible with
  /// its jump disabled, and the fixture says it is one.
  import {
    Sidebar,
    SidebarContent,
    SidebarGroup,
    SidebarGroupLabel,
    SidebarHeader,
    SidebarMenu,
    SidebarMenuButton,
    SidebarMenuItem,
    SidebarRail,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { t } from "$lib/i18n/messages";
  import { pdfMocked, type DocumentInfo, type SearchOutcome } from "$lib/stores/pdf";

  let {
    doc,
    query = $bindable(""),
    results,
    current,
    onjump,
  }: {
    doc: DocumentInfo;
    query: string;
    results: SearchOutcome | null;
    current: number;
    onjump: (page: number) => void;
  } = $props();

  /// The search word emphasised inside its snippet, as three parts.
  function splitSnippet(snippet: string): { before: string; match: string; after: string } {
    const q = query.trim();
    const at = q ? snippet.toLowerCase().indexOf(q.toLowerCase()) : -1;
    if (at < 0) return { before: snippet, match: "", after: "" };
    return { before: snippet.slice(0, at), match: snippet.slice(at, at + q.length), after: snippet.slice(at + q.length) };
  }

  /// Which outline entry the current page sits under (the last one at or
  /// before it), for the active marker.
  const activeIndex = $derived.by(() => {
    let best = -1;
    doc.outline.forEach((e, i) => {
      if (e.page !== null && e.page <= current) best = i;
    });
    return best;
  });
</script>

<Sidebar>
  <SidebarHeader class="h-10 flex-row items-center py-0">
    <span class="px-2 text-[0.6875rem] font-semibold uppercase tracking-[0.1em] text-sidebar-foreground/55">
      {$t("pdf.app.title")}
    </span>
  </SidebarHeader>
  <SidebarContent>
    <SidebarGroup>
      <SearchField id="pdf-search" bind:value={query} placeholder={$t("pdf.search.label")} aria-label={$t("pdf.search.label")} />
      {#if $pdfMocked}
        <p class="side-note">{$t("pdf.sample")}</p>
      {/if}
    </SidebarGroup>

    {#if results}
      <SidebarGroup class="pt-0">
        {#if results.hits.length === 0}
          <p class="side-note">{$t("pdf.search.none")}</p>
        {:else}
          <ul class="pdf-hits">
            {#each results.hits as hit (hit.page)}
              {@const parts = splitSnippet(hit.snippet)}
              <li>
                <button type="button" class="hit" onclick={() => onjump(hit.page)}>
                  <span class="hit-page">{$t("pdf.page", { number: hit.page })}</span>
                  <span class="hit-text">{parts.before}{#if parts.match}<mark>{parts.match}</mark>{/if}{parts.after}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
        <!-- Said whether or not there were hits: pages nobody could read are
             missing from BOTH answers, and the empty one is where a reader is
             most likely to conclude the word is not in the document. -->
        {#if results.unsearchable.length > 0}
          <p class="side-note">{$t("pdf.search.unsearchable", { count: results.unsearchable.length })}</p>
        {/if}
      </SidebarGroup>
    {:else}
      <SidebarGroup class="pt-0">
        <SidebarGroupLabel>{$t("pdf.contents")}</SidebarGroupLabel>
        {#if doc.outline.length === 0}
          <p class="side-note">{$t("pdf.noContents")}</p>
        {:else}
          <SidebarMenu class="pdf-contents">
            {#each doc.outline as entry, i (i)}
              <SidebarMenuItem>
                <!-- An entry whose target this reader could not resolve is
                     still shown, with the jump disabled. Hiding it would lose
                     a heading the document plainly has. -->
                <SidebarMenuButton
                  isActive={i === activeIndex}
                  disabled={entry.page === null}
                  class="min-h-7 h-auto py-1"
                  style="padding-inline-start: {8 + entry.depth * 14}px"
                  onclick={() => entry.page && onjump(entry.page)}
                >
                  <span class="truncate">{entry.title}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
            {/each}
          </SidebarMenu>
        {/if}
      </SidebarGroup>
    {/if}
  </SidebarContent>
  <SidebarRail />
</Sidebar>

<style>
  .side-note {
    margin: 6px 8px 2px;
    font-size: 11px;
    line-height: 1.4;
    color: color-mix(in srgb, currentColor 55%, transparent);
  }
  .pdf-hits {
    list-style: none;
    margin: 0;
    padding: 0 0.25rem;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .hit {
    display: flex;
    width: 100%;
    flex-direction: column;
    gap: 1px;
    padding: 0.35rem 0.45rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font: inherit;
    text-align: start;
    color: inherit;
    cursor: pointer;
  }
  .hit:hover {
    background: color-mix(in srgb, currentColor 6%, transparent);
  }
  .hit:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: -2px;
  }
  .hit-page {
    font-size: 11px;
    color: color-mix(in srgb, currentColor 50%, transparent);
  }
  .hit-text {
    font-size: 12px;
    line-height: 1.4;
    overflow-wrap: anywhere;
  }
  .hit-text mark {
    background: color-mix(in srgb, var(--color-accent, #6366f1) 35%, transparent);
    color: inherit;
    border-radius: 2px;
  }
</style>
