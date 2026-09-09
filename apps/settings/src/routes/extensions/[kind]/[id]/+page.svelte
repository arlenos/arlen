<script lang="ts">
  /// One extension's page: what it is, whether it runs, what it may reach set
  /// against what this machine has seen it do, the controls its kind really
  /// has, and the one place to take its authority back.
  ///
  /// Declared is not observed, and the page says so with two words rather than
  /// one list: "What it may reach" carries the manifest, and beside each line
  /// the audit ledger's answer as a value. A module is never measured, because
  /// nothing audits its host calls; that is said once under the section, not
  /// hidden behind a confident silence.
  ///
  /// The revoke is a shape, not a dialog (design-system.md 6.2: a revoke only
  /// narrows, so no hard confirm): what goes and what stays are written above
  /// the one button, and the report lands where the button was.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { AppWindow, Puzzle, Cable } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { t, locale } from "$lib/i18n/messages";
  import {
    extensions,
    inventoryState,
    inventoryMocked,
    loadExtensions,
    loadObserved,
    revokeExtension,
    kindOf,
    type DeclaredCapability,
    type RevokeReport,
  } from "$lib/stores/extensions";
  import { modules } from "$lib/stores/modules";
  import { ago, capSentence, failureReason, healthReadout, originLine } from "$lib/extensionWords";

  const kind = $derived(kindOf($page.params.kind ?? ""));
  const id = $derived(decodeURIComponent($page.params.id ?? ""));
  const ext = $derived(kind ? $extensions.find((e) => e.kind === kind && e.id === id) : undefined);
  const ICONS = { app: AppWindow, module: Puzzle, bridge: Cable } as const;

  let seen = $state<DeclaredCapability[] | null | undefined>(undefined);
  let report = $state<RevokeReport | null>(null);
  let revokeRefused = $state(false);
  let busy = $state(false);

  onMount(() => {
    void (async () => {
      if ($extensions.length === 0) await loadExtensions();
      if (ext) seen = await loadObserved(ext);
      if (ext?.kind === "module") void modules.load();
    })();
  });

  /// The observation as a value beside the capability, with its posture.
  function seenReadout(cap: DeclaredCapability) {
    const o = cap.observation;
    if (o.state === "observed") return { posture: "ours", text: $t("s.ext.seen.used", { when: ago(o.lastMicros, $locale, $t) }) };
    if (o.state === "notObserved") return { posture: "off", text: $t("s.ext.seen.notLately") };
    return { posture: "unknown", text: $t("s.ext.seen.notMeasured") };
  }
  /// The one reason under the section when something was not measured; the
  /// first reason found, since a mixed answer names the mechanism that is missing.
  const unmeasuredWhy = $derived.by(() => {
    const r = (seen ?? []).find((c) => c.observation.state === "notMeasured");
    if (!r || r.observation.state !== "notMeasured") return null;
    return $t(`s.ext.unmeasured.${r.observation.reason}`);
  });

  /// The module's switch reads the module store where it answers (live) and
  /// the inventory's own health where it does not (the sample), so the control
  /// is the same shape either way.
  const moduleRow = $derived(ext?.kind === "module" ? $modules.data.find((m) => m.id === ext.id) : undefined);
  const moduleEnabled = $derived(moduleRow ? moduleRow.enabled : ext?.health !== "disabled");

  async function setModuleEnabled(enabled: boolean) {
    if (!ext) return;
    if ($inventoryMocked) {
      extensions.update((all) =>
        all.map((e) => (e.kind === ext.kind && e.id === ext.id ? { ...e, health: enabled ? "active" : "disabled" } : e)),
      );
      return;
    }
    await modules.setEnabled(ext.id, enabled);
    await loadExtensions();
  }

  async function uninstallModule() {
    if (!ext) return;
    if ($inventoryMocked) extensions.update((all) => all.filter((e) => !(e.kind === ext.kind && e.id === ext.id)));
    else await modules.uninstall(ext.id);
    await goto("/extensions");
  }

  async function revoke() {
    if (!ext || busy) return;
    busy = true;
    revokeRefused = false;
    const r = await revokeExtension(ext);
    busy = false;
    if (r === null) revokeRefused = true;
    else report = r;
  }

  /// The residue the plan carries, said before the press: the same sentences
  /// the host reports afterwards, because what stays is known from the kind.
  const residue = $derived.by(() => {
    if (!ext) return [];
    if (ext.kind === "bridge") {
      return [$t("s.ext.residue.bridgeStops", { name: ext.name }), $t("s.ext.residue.bridgeKeeps", { namespace: ext.id })];
    }
    return [$t("s.ext.residue.keeps", { name: ext.name })];
  });
</script>

{#if !kind}
  <Page back={{ href: "/extensions", label: $t("s.nav.extensions") }} title={$t("s.ext.title")}>
    <SectionGrid><Notice tone="error" class="span-full" text={$t("s.ext.notFound")} /></SectionGrid>
  </Page>
{:else if $inventoryState === "loading"}
  <Page back={{ href: "/extensions", label: $t("s.nav.extensions") }} title={$t("s.ext.title")}>
    <SectionGrid><p class="quiet span-full">{$t("s.ext.loading")}</p></SectionGrid>
  </Page>
{:else if !ext}
  <Page back={{ href: "/extensions", label: $t("s.nav.extensions") }} title={$t("s.ext.title")}>
    <SectionGrid>
      <Notice tone="error" class="span-full" text={$t("s.ext.notFound")} />
      <div class="span-full"><Button variant="outline" size="sm" onclick={() => goto("/extensions")}>{$t("s.ext.backToList")}</Button></div>
    </SectionGrid>
  </Page>
{:else}
  {@const Icon = ICONS[ext.kind]}
  {@const health = healthReadout(ext.health, $t)}
  {@const why = failureReason(ext.health)}
  <Page back={{ href: "/extensions", label: $t("s.nav.extensions") }} title={ext.name} description={originLine(ext, $t)}>
    <SectionGrid>
      {#if $inventoryMocked}
        <Notice tone="neutral" class="span-full" text={$t("s.ext.sample")} />
      {/if}

      <Section class="span-full">
        <Row label={$t("s.ext.state")} description={ext.id}>
          {#snippet leading()}<span class="kind-icon"><Icon size={16} strokeWidth={1.75} /></span>{/snippet}
          {#snippet control()}
            <span class="readout" class:away={health.posture === "away"}>
              <span class="found" data-posture={health.posture} aria-hidden="true"></span>{health.text}
            </span>
          {/snippet}
        </Row>
        {#if why}
          <div class="in-card"><Notice tone="error" text={$t("s.ext.failedWhy", { why })} /></div>
        {/if}
        {#if ext.kind === "module"}
          <Row label={$t("s.ext.module.enabled")} description={$t("s.ext.module.restart")}>
            {#snippet control()}
              <Switch value={moduleEnabled} ariaLabel={$t("s.ext.module.enabled")} onchange={setModuleEnabled} />
            {/snippet}
          </Row>
          <Row label={$t("s.ext.module.remove")} description={$t("s.ext.module.removeDesc")}>
            {#snippet control()}
              <Button variant="outline" size="sm" onclick={uninstallModule}>{$t("s.mod.uninstall")}</Button>
            {/snippet}
          </Row>
        {/if}
      </Section>

      <Section label={$t("s.ext.mayReach")} class="span-full">
        {#if ext.capabilities.length === 0}
          <p class="quiet in-card">{$t("s.ext.cap.none")}</p>
        {:else}
          {#each ext.capabilities as cap (cap)}
            {@const s = seen?.find((c) => c.label === cap)}
            <Row label={capSentence(cap, $t)}>
              {#snippet control()}
                {#if s}
                  {@const r = seenReadout(s)}
                  <span class="readout">
                    <span class="found" data-posture={r.posture} aria-hidden="true"></span>{r.text}
                  </span>
                {/if}
              {/snippet}
            </Row>
          {/each}
          {#if seen === null}
            <p class="caveat in-card">{$t("s.ext.unmeasured.ledgerUnavailable")}</p>
          {:else if unmeasuredWhy}
            <p class="caveat in-card">{unmeasuredWhy}</p>
          {/if}
        {/if}
      </Section>

      {#if ext.kind === "app"}
        <LinkCard href={`/apps/${encodeURIComponent(ext.id)}`} title={$t("s.ext.app.page")} description={$t("s.ext.app.pageDesc")} />
        <LinkCard href="/privacy" title={$t("s.ext.app.access")} description={$t("s.ext.app.accessDesc")} />
      {/if}

      {#if ext.capabilities.length > 0}
        <Section label={$t("s.ext.takeBack")} class="span-full">
          {#if report}
            <div class="in-card report">
              {#each report.revoked as line (line)}
                <p class="gone"><span class="found" data-posture="ours" aria-hidden="true"></span>{line}</p>
              {/each}
              {#each report.failed as line (line)}
                <Notice tone="error" text={line} />
              {/each}
              {#each report.residue as line (line)}
                <p class="quiet">{line}</p>
              {/each}
            </div>
          {:else}
            <div class="in-card cost">
              <p class="cost-lead">{$t("s.ext.goes")}</p>
              <ul class="cost-list">
                {#each ext.capabilities as cap (cap)}
                  <li>{capSentence(cap, $t)}</li>
                {/each}
              </ul>
              {#each residue as line (line)}
                <p class="quiet">{line}</p>
              {/each}
              {#if revokeRefused}
                <Notice tone="error" text={$t("s.ext.revokeRefused")} />
              {/if}
              <div class="actions">
                <Button variant="destructive" size="sm" disabled={busy} onclick={revoke}>{$t("s.ext.revokeAll")}</Button>
              </div>
            </div>
          {/if}
        </Section>
      {/if}
    </SectionGrid>
  </Page>
{/if}

<style>
  .quiet {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .in-card {
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .caveat {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .kind-icon {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .readout {
    display: inline-block;
    max-width: clamp(7rem, 22vw, 20rem);
    font-size: var(--text-xs);
    line-height: 1.35;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    text-align: end;
  }
  .readout.away {
    color: var(--color-warning);
  }
  .found {
    display: inline-block;
    vertical-align: middle;
    margin-inline-end: 0.45rem;
    width: 6px;
    height: 6px;
    border-radius: var(--radius-chip);
  }
  .found[data-posture="ours"] {
    background: var(--color-success);
  }
  .found[data-posture="off"] {
    background: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .found[data-posture="away"] {
    background: var(--color-warning);
  }
  .found[data-posture="unknown"] {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
  }
  /* The cost, said before the button: what goes as a list, what stays as
     quiet lines, then the one button under all of it. */
  .cost {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .cost-lead {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--foreground);
  }
  .cost-list {
    margin: 0;
    padding-inline-start: 1.1rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .report {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .gone {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--foreground);
  }
</style>
