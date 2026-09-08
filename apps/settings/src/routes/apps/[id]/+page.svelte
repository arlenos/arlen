<script lang="ts">
  /// One app's page (per-app-settings-plan.md §4): identity first, then the
  /// app's own declared settings rendered by the schema -> widget pipeline,
  /// then what it can reach (the same PrincipalGrants component as the privacy
  /// browser, required reaches demoted below the revocable ones), provenance
  /// last. An unverified publisher banners the whole page before anything
  /// below it is read. No declared schema shows honestly as one quiet line,
  /// never an invented panel.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/stores";
  import { formatDecimal } from "@arlen/ui-kit/i18n";
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
    appMeta,
    appMetaMocked,
    loadAppMeta,
    appGeneral,
    loadAppGeneral,
    clearAppCache,
  } from "$lib/stores/appSettings";
  import { appAudit, loadAppAudit } from "$lib/stores/appAudit";
  import { orderedSections, orphanKeys } from "$lib/appSettings";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import PrincipalGrants from "$lib/components/privacy/PrincipalGrants.svelte";
  import SchemaSection from "$lib/components/apps/SchemaSection.svelte";
  import { t, locale } from "$lib/i18n/messages";

  const appId = $derived($page.params.id ?? "");

  onMount(loadGrants);
  $effect(() => {
    if (appId) {
      loadAppSettings(appId);
      loadAppMeta(appId);
      loadAppGeneral(appId);
      loadAppAudit(appId);
    }
  });

  const principal = $derived(byApp($t, $locale, $grants).find((p) => p.appId === appId));
  const label = $derived(principal?.label ?? appId);
  const unverified = $derived(principal ? !principal.identityVerified : false);
  const sections = $derived($appPage ? orderedSections($appPage.schema) : []);
  const orphans = $derived($appPage ? orphanKeys($appPage) : []);
  const mocked = $derived($appPageMocked || $grantsMocked || $appMetaMocked);

  // Version and publisher share one quiet identity line; either may be absent.
  const metaLine = $derived([$appMeta?.version, $appMeta?.publisher].filter(Boolean).join(", "));

  // The number is the reader's: `toFixed` writes a decimal POINT in every
  // language, so this said "1.5 GB" to somebody whose machine writes "1,5 GB".
  // The space before the unit is a no-break one - a size is one quantity.
  function fmtBytes(n: number): string {
    if (n < 1_000_000) return `${formatDecimal(Math.max(1, Math.round(n / 1_000)))}\u00a0kB`;
    if (n < 1_000_000_000) return `${formatDecimal(Math.round(n / 1_000_000))}\u00a0MB`;
    return `${formatDecimal(n / 1_000_000_000, 1)}\u00a0GB`;
  }

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
  /// Uninstalling. `settings_app_uninstall` has been implemented and registered
  /// for weeks; the button beside it was disabled under a comment saying there
  /// was no command yet, which is the stale-note shape this tree keeps finding.
  ///
  /// The refusal is the daemon's own sentence. It decides what may be removed -
  /// a component of the running desktop is refused BY NAME - and that wording is
  /// the only part the person can act on, so it is shown rather than wrapped.
  let uninstallError = $state<string | null>(null);
  function askUninstall(appLabel: string) {
    uninstallError = null;
    pending = {
      title: $t("s.apps.uninstall.title"),
      message: $t("s.apps.uninstall.msg", { app: appLabel }),
      run: async () => {
        try {
          await invoke("settings_app_uninstall", { appId });
          await goto("/apps");
        } catch (e) {
          // THE COMMAND ANSWERS A TOKEN, so every sentence a person reads is
          // written here. It used to answer a sentence, which meant this page
          // finished a German line with the daemon's English one - what
          // `check-refusal-language` refuses, and it is right that one place
          // chooses the language. The detail is for the log.
          //
          // `unknown` earns its own kind rather than folding into a failure: if
          // the daemon stops reporting mid-job, whether the app is gone is
          // genuinely not known, and saying "it was not removed" would be a
          // guess dressed as a fact.
          console.error("uninstall failed", e);
          const kind = (e as { kind?: string } | null)?.kind;
          uninstallError =
            kind === "unavailable"
              ? $t("s.apps.uninstall.unavailable")
              : kind === "refused"
                ? $t("s.apps.uninstall.refused")
                : kind === "unknown"
                  ? $t("s.apps.uninstall.unknown")
                  : $t("s.apps.uninstall.failed");
        }
      },
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
        {#if metaLine}
          <span class="head-meta">{metaLine}</span>
        {/if}
      </div>
      <div class="head-actions">
        <!-- Open is gone rather than disabled. Settings has no way to launch
             another app - no shell IPC, no spawn - so the control could never do
             anything, and a button that can never work is chrome wearing a
             control's clothes. The launcher opens apps. -->
        <Button variant="outline" size="sm" onclick={() => askUninstall(label)}>
          {$t("s.apps.uninstall")}
        </Button>
      </div>
    </div>

    {#if uninstallError}
      <p class="uninstall-error span-full" role="alert">{uninstallError}</p>
    {/if}

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
      <div class="sect span-full">
        <Section label={$t("s.apps.reach")}>
          <PrincipalGrants {principal} split showHead={false} onRemoveScope={askScope} />
        </Section>
        <!-- The declared half. The observed half is the section below it. -->
        <p class="sect-desc">{$t("s.apps.usageNote")}</p>
      </div>
      <!-- What it actually did, from the audit ledger, filtered daemon-side to
           this app's kernel-attested actor. The three states are kept apart: a
           record that cannot be read is not a record of nothing. -->
      <div class="sect span-full">
        <Section label={$t("s.apps.activity")}>
          {#if $appAudit === null}
            <p class="no-schema">{$t("s.apps.activityReading")}</p>
          {:else if !$appAudit.available}
            <p class="no-schema">{$t("s.apps.activityUnavailable")}</p>
          {:else if $appAudit.entries.length === 0}
            <p class="no-schema">{$t("s.apps.activityNone")}</p>
          {:else}
            {#if $appAudit.tampered}
              <p class="no-schema" role="alert">{$t("s.apps.activityTampered")}</p>
            {/if}
            {#each $appAudit.entries as e (e.index)}
              <Row
                label={$t(`s.apps.did.${e.kind}`)}
                description={`${e.subject} - ${e.outcome}`}
              />
            {/each}
          {/if}
        </Section>
        {#if $appAudit && $appAudit.available && $appAudit.total > $appAudit.entries.length}
          <p class="sect-desc">
            {$t("s.apps.activityMore", { total: $appAudit.total })}
          </p>
        {/if}
      </div>
      <div class="span-full">
        <LinkCard href="/privacy" title={$t("s.apps.allApps")} description={$t("s.apps.allAppsDesc")}>
          {#snippet icon()}<Shield size={20} strokeWidth={1.75} />{/snippet}
        </LinkCard>
      </div>
    {/if}

    {#if $appGeneral}
      {@const g = $appGeneral}
      <Section label={$t("s.apps.general")} class="span-full">
        {#if g.opens.length > 0}
          <Row label={$t("s.apps.opens")} description={g.opens.join(", ")} />
        {/if}
        {#if g.appBytes != null || g.cacheBytes != null}
          <Row
            id={`${appId}.storage`}
            label={$t("s.apps.storage")}
            description={$t("s.apps.storageVal", {
              app: fmtBytes(g.appBytes ?? 0),
              cache: fmtBytes(g.cacheBytes ?? 0),
            })}
          >
            {#snippet control()}
              <Button variant="outline" size="sm" onclick={() => clearAppCache(appId)}>
                {$t("s.apps.clearCache")}
              </Button>
            {/snippet}
          </Row>
        {/if}
        {#if g.defaultFor.length > 0}
          <Row label={$t("s.apps.defaultFor")} description={g.defaultFor.join(", ")} />
        {/if}
      </Section>
    {/if}

    {#if $appPage || $appMeta}
      <Section label={$t("s.apps.aboutApp")} class="span-full">
        <Row
          label={$t("s.apps.source")}
          description={$appMeta?.storeComponent ? $t("s.apps.sourceStore") : $t("s.apps.sourceLocal")}
        />
        {#if $appPage}
          <Row
            label={$t("s.apps.schemaVersion")}
            description={$t("s.apps.schemaVersionVal", { n: $appPage.schema.version })}
          />
        {/if}
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
  .uninstall-error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--destructive);
  }
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
    color: var(--color-fg-secondary, #a1a1aa);
  }
  .head-meta {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
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
