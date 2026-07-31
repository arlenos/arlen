<script lang="ts">
  /// One app's page (per-app-settings-plan.md §4): identity first, then the
  /// app's own declared settings rendered by the schema -> widget pipeline,
  /// then what it can reach (the same PrincipalGrants component as the privacy
  /// browser, required reaches demoted below the revocable ones), provenance
  /// last. An unverified publisher banners the whole page before anything
  /// below it is read. No declared schema shows honestly as one quiet line,
  /// never an invented panel.
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { ShieldAlert, Shield } from "lucide-svelte";
  import {
    grants,
    grantsMocked,
    byApp,
    loadGrants,
    revokeScope,
    type ScopeLine,
  } from "$lib/stores/grants";
  import {
    appPage,
    appPageMocked,
    writeErrors,
    loadAppSettings,
  } from "$lib/stores/appSettings";
  import { orderedSections, orphanKeys } from "$lib/appSettings";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import PrincipalGrants from "$lib/components/privacy/PrincipalGrants.svelte";
  import SchemaSection from "$lib/components/apps/SchemaSection.svelte";
  import { t } from "$lib/i18n/messages";

  const appId = $derived($page.params.id ?? "");

  onMount(loadGrants);
  $effect(() => {
    if (appId) loadAppSettings(appId);
  });

  const principal = $derived(byApp($grants).find((p) => p.appId === appId));
  const label = $derived(principal?.label ?? appId);
  const unverified = $derived(principal ? !principal.identityVerified : false);
  const sections = $derived($appPage ? orderedSections($appPage.schema) : []);
  const orphans = $derived($appPage ? orphanKeys($appPage) : []);
  const mocked = $derived($appPageMocked || $grantsMocked);

  // The per-line remove, same confirm as the privacy browser; the full
  // management surface (undo, remove-all, by-data) stays over there.
  let pending = $state<{ title: string; message: string; run: () => Promise<unknown> } | null>(null);
  function askScope(appLabel: string, line: ScopeLine) {
    pending = {
      title: $t("s.priv.askScope.title"),
      message: $t("s.priv.askScope.msg", { what: line.text, app: appLabel }),
      run: () => revokeScope(line, appLabel),
    };
  }
  async function onConfirm() {
    if (pending === null) return;
    await pending.run();
    pending = null;
  }
</script>

<!-- The identity head is the page's hero; a Page title above it would just
     repeat the name. -->
<Page>
  <SectionGrid>
    {#if mocked}
      <p class="note span-full">{$t("s.apps.sample")}</p>
    {/if}

    <div class="head span-full">
      <AppAvatar {appId} {label} size={48} />
      <div class="head-text">
        <span class="head-name">{label}</span>
        {#if label !== appId}
          <span class="head-id">{appId}</span>
        {/if}
      </div>
      <div class="head-actions">
        <!-- Launch and uninstall have no Settings command yet (seams); the
             buttons state the shape of the page without pretending to work. -->
        <Button variant="outline" size="sm" disabled>{$t("s.apps.open")}</Button>
        <Button variant="outline" size="sm" disabled>{$t("s.apps.uninstall")}</Button>
      </div>
    </div>

    {#if unverified}
      <div class="banner span-full" role="note">
        <ShieldAlert size={16} strokeWidth={1.75} />
        <span>{$t("s.apps.unverifiedBanner")}</span>
      </div>
    {/if}

    {#if $appPage}
      {#each sections as section (section.label)}
        <SchemaSection
          {appId}
          {section}
          values={$appPage.values}
          userSet={$appPage.userSet}
          unavailable={$appPage.unavailable}
          errors={$writeErrors}
        />
      {/each}

      {#if orphans.length > 0}
        <div class="sect span-full">
          <Section label={$t("s.apps.olderVersion")}>
            {#each orphans as key (key)}
              <Row id={`${appId}.${key}`} label={key} description={String($appPage.values[key])} />
            {/each}
          </Section>
          <p class="sect-desc">{$t("s.apps.olderVersionNote")}</p>
        </div>
      {/if}
    {:else}
      <Section class="span-full">
        <p class="no-schema">{$t("s.apps.noSchema")}</p>
      </Section>
    {/if}

    {#if principal}
      <Section label={$t("s.apps.reach")} class="span-full">
        <PrincipalGrants {principal} split showHead={false} onRemoveScope={askScope} />
      </Section>
      <div class="span-full">
        <LinkCard href="/privacy" title={$t("s.apps.allApps")} description={$t("s.apps.allAppsDesc")}>
          {#snippet icon()}<Shield size={20} strokeWidth={1.75} />{/snippet}
        </LinkCard>
      </div>
    {/if}

    {#if $appPage}
      <Section label={$t("s.apps.aboutApp")} class="span-full">
        <Row
          label={$t("s.apps.schemaVersion")}
          description={$t("s.apps.schemaVersionVal", { n: $appPage.schema.version })}
        />
      </Section>
    {/if}
  </SectionGrid>
</Page>

<ConfirmDialog
  open={pending !== null}
  title={pending?.title ?? ""}
  message={pending?.message ?? ""}
  confirmLabel={$t("s.priv.remove")}
  variant="destructive"
  {onConfirm}
  onCancel={() => (pending = null)}
/>

<style>
  .note {
    margin: 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* Identity head: the app's mark and name anchor the page; the actions sit
     pinned to the right like a desktop detail view, not inline links. */
  .head {
    display: flex;
    align-items: center;
    gap: 1rem;
  }
  .head-text {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    min-width: 0;
  }
  .head-name {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--foreground);
  }
  .head-id {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .head-actions {
    margin-inline-start: auto;
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  /* The unverified banner qualifies everything below it, so it sits above
     everything and keeps the warning register (never red alarm). */
  .banner {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    padding: 0.625rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-warning, #ca8a04) 35%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-warning, #ca8a04) 8%, transparent);
    font-size: var(--text-sm);
    color: var(--foreground);
  }
  .banner :global(svg) {
    flex-shrink: 0;
    color: var(--color-warning, #ca8a04);
  }

  .no-schema {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .sect {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .sect-desc {
    margin: 0;
    padding-inline-start: 0.25rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
