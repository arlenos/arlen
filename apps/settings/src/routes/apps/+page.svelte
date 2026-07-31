<script lang="ts">
  /// The installed-apps list (per-app-settings-plan.md): one calm row per app,
  /// leading to its page of declared settings + access. Today the list is
  /// honestly derived from the apps the grant ledger knows (there is no
  /// `settings_apps_list` bridge yet; name, version and publisher would come
  /// from the shell's app_index - flagged as a seam). The assistant principals
  /// stay on the privacy page; this list is about apps.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { grants, grantsLoaded, grantsMocked, byApp, loadGrants } from "$lib/stores/grants";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import { t } from "$lib/i18n/messages";

  onMount(loadGrants);

  const apps = $derived(byApp($grants).filter((p) => !p.assistant));
</script>

<Page title={$t("s.apps.title")} description={$t("s.apps.desc")}>
  <SectionGrid>
    {#if $grantsMocked}
      <p class="note span-full">{$t("s.apps.sample")}</p>
    {/if}

    {#if $grantsLoaded && apps.length === 0}
      <Group class="span-full">
        <p class="note pad">{$t("s.apps.empty")}</p>
      </Group>
    {:else if apps.length > 0}
      <Group class="span-full">
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
      </Group>
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
    cursor: pointer;
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
    margin-left: 0.375rem;
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
