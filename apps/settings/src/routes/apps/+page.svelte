<script lang="ts">
  /// The installed-apps list (per-app-settings-plan.md): one calm row per app,
  /// leading to its page of declared settings + access.
  ///
  /// THE ROWS ARE THE INSTALLED APPS, and until 5 September they were the grant
  /// ledger, which is a different set. `settings_apps_list` had been implemented
  /// and registered for a while and nothing called it; the note here still said
  /// there was no bridge yet. An app that ships a settings schema and holds no
  /// grant has settings all the same, and a page nobody can reach looks exactly
  /// like an app that has none. A grant is a property of a row now, not the
  /// reason one exists. `mergeAppRows` keeps granted-but-not-installed rows too,
  /// so the fix takes nothing away.
  ///
  /// The assistant principals stay on the privacy page; this list is about apps.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { grants, grantsLoaded, grantsMocked, grantsError, byApp, loadGrants } from "$lib/stores/grants";
  import {
    installed,
    installedLoaded,
    installedError,
    loadInstalledApps,
    mergeAppRows,
  } from "$lib/stores/installedApps";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import { t, locale } from "$lib/i18n/messages";

  onMount(() => {
    void loadGrants();
    void loadInstalledApps();
  });

  const granted = $derived(byApp($t, $locale, $grants).filter((p) => !p.assistant));
  const apps = $derived(mergeAppRows($installed, granted, new Intl.Collator($locale)));
  const ready = $derived($grantsLoaded && $installedLoaded);
  /// Both reads failing is the only case that can say nothing at all. One of the
  /// two failing still leaves a real list, so the page shows it rather than
  /// refusing over a source the reader was not asking about.
  const bothFailed = $derived($grantsError && $installedError);
</script>

<Page title={$t("s.apps.title")} description={$t("s.apps.desc")}>
  <SectionGrid>
    {#if $grantsMocked}
      <p class="note span-full">{$t("s.apps.sample")}</p>
    {/if}

    <!-- The installed read failed while the ledger answered. The list below is
         real and SHORT: it holds only the apps that hold a grant, which is the
         set this page stopped being. Without this line a partial list is
         indistinguishable from a complete one. -->
    {#if $installedError && !bothFailed}
      <p class="note span-full">{$t("s.apps.partial")}</p>
    {/if}

    {#if !ready && apps.length === 0}
      <!-- The read is quick, but a blank page reads as broken; one quiet line. -->
      <Section class="span-full">
        <p class="note pad">{$t("s.apps.loading")}</p>
      </Section>
    {:else if bothFailed}
      <!-- Both reads failed. Not the same as an empty machine: saying "no app has
           access" when we could not ask is the one sentence this page must never
           get wrong. -->
      <Section class="span-full">
        <p class="note pad">{$t("s.apps.unavailable")}</p>
      </Section>
    {:else if ready && apps.length === 0}
      <Section class="span-full">
        <p class="note pad">{$t("s.apps.empty")}</p>
      </Section>
    {:else if apps.length > 0}
      <Section class="span-full">
        {#each apps as p (p.appId)}
          <button type="button" class="app-row" id={`app-${p.appId}`} onclick={() => goto(`/apps/${p.appId}`)}>
            <AppAvatar appId={p.appId} label={p.label} size={32} />
            <span class="app-name">
              {p.label}
              {#if !p.identityVerified}<span class="warn">{$t("s.priv.unverified")}</span>{/if}
            </span>
            {#if p.label !== p.appId}
              <span class="app-id">{p.appId}</span>
            {/if}
            <span class="chev"><ChevronRight size={15} strokeWidth={2} /></span>
          </button>
        {/each}
      </Section>
    {/if}
  </SectionGrid>
</Page>

<style>
  .note {
    margin: 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .note.pad {
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
  }

  /* One row per app inside the Group card (the card draws the dividers). */
  .app-row {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: 100%;
    padding: var(--space-row, 0.75rem) 1rem;
    border: none;
    background: transparent;
    text-align: start;
    transition: background var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .app-row:hover {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .app-name {
    flex: 1;
    min-width: 0;
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .warn {
    margin-inline-start: 0.375rem;
    font-size: var(--text-2xs);
    color: var(--color-warning, #ca8a04);
  }
  .app-id {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .chev {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
</style>
