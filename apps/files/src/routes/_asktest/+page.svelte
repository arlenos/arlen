<script lang="ts">
  /// Headless render harness for "Ask Arlen". UI-AFFORDANCE verification ONLY,
  /// NOT a behaviour claim. Mocks the daemon over Tauri IPC (the scoped ask +
  /// the off-switch read) and renders the real search bar (Search/Ask toggle),
  /// the drafted facet chips, the "Arlen drafted this" banner with the reads
  /// line, and a stand-in result listing. Type an ask and press Enter to draft.
  /// `?state=off` greys out Ask (assistant disabled). The real scoped ask is the
  /// coder's `files_ask`. Dev route only.
  import { onMount } from "svelte";
  import FmSearchBar from "$lib/components/FmSearchBar.svelte";
  import FmAskBanner from "$lib/components/FmAskBanner.svelte";
  import FmFacetBar from "$lib/components/FmFacetBar.svelte";
  import { searchOpen } from "$lib/stores/search";
  import { facetOpen, loadFacetOptions, clearFacets } from "$lib/stores/facets";
  import {
    askMode,
    askDraft,
    runAsk,
    applyDraft,
    clearAsk,
    loadAiEnabled,
  } from "$lib/stores/ask";

  const FOLDER = "/home/tim/projects/arlen";

  // The stand-in faceted result (the live listing is the preview in the app).
  const RESULT = [
    { name: "parser.rs", where: "ai-agent/src", modified: "2 days ago" },
    { name: "lexer.rs", where: "ai-core/src", modified: "5 days ago" },
    { name: "tokens.rs", where: "ai-core/src", modified: "9 days ago" },
  ];

  let failed = $state(false);

  async function ask(query: string) {
    failed = false;
    const result = await runAsk(FOLDER, query);
    if (!result) {
      failed = true;
      return;
    }
    applyDraft(result, query);
  }

  function dismiss() {
    clearAsk();
    clearFacets();
  }

  let ready = $state(false);
  onMount(async () => {
    const off = new URLSearchParams(window.location.search).get("state") === "off";
    const { mockIPC } = await import("@tauri-apps/api/mocks");
    mockIPC((cmd) => {
      if (cmd === "files_ai_enabled") return !off;
      if (cmd === "files_projects")
        return [{ id: "p-arlen", name: "Arlen OS", path: "/home/tim/projects/arlen" }];
      if (cmd === "files_touched_apps") return [{ id: "files", label: "Files", count: 12 }];
      if (cmd === "files_ask")
        return {
          facets: { type: ["code"], time: ["week"], project: ["p-arlen"] },
          reads: { files: 487, tags: 3 },
        };
      return null;
    });
    await loadAiEnabled();
    await loadFacetOptions();
    askMode.set("ask");
    searchOpen.set(true);
    ready = true;
  });
</script>

<div class="harness">
  {#if ready}
    <FmSearchBar path={FOLDER} onask={ask} />
    {#if failed}
      <div class="ask-fallback">
        Arlen could not draft a filter for that. Try naming a type, a time, or a
        project.
      </div>
    {/if}
    {#if $facetOpen}
      {#if $askDraft}
        <FmAskBanner scope={FOLDER} ondismiss={dismiss} />
      {/if}
      <FmFacetBar basePath={FOLDER} onnavigate={() => {}} />
      <div class="listing">
        <div class="lhead"><span>Name</span><span>Location</span><span>Modified</span></div>
        {#each RESULT as r (r.name)}
          <div class="lrow">
            <span class="lname">{r.name}</span>
            <span class="lwhere">{r.where}</span>
            <span class="lmod">{r.modified}</span>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .harness {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--background);
    color: var(--foreground);
  }
  .ask-fallback {
    padding: 8px 12px;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .listing {
    font-size: 0.8125rem;
  }
  .lhead,
  .lrow {
    display: grid;
    grid-template-columns: 1fr 1fr 120px;
    gap: 12px;
    padding: 6px 16px;
    align-items: center;
  }
  .lhead {
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-size: 0.6875rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .lrow + .lrow {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .lname {
    color: var(--foreground);
  }
  .lwhere,
  .lmod {
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
</style>
