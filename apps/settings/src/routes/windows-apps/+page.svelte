<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Windows apps / Compatibility (windows-apps-plan.md). Windows apps run in a
  /// managed compatibility layer that is auto-configured for known apps, so this
  /// page keeps only what a glance needs: each installed app with its honest
  /// compat tier (curated-verified vs best-effort, never "just works") and a
  /// Launch, plus the install entry and what this machine can run. Everything
  /// deeper lives on the app's own page - shallow by default, deep on demand.
  ///
  /// Every state the page can be in is said in the house register: a page-level
  /// fact or refusal is a Notice above the cards, and an empty list is not a card
  /// saying "empty" but the install row stepping forward as the one thing to do.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import {
    winApps,
    winActionFailed,
    launchFailed,
    installFailed,
    installFailureKey,
    launchFailureKey,
    forgetFailed,
    forgetFailureKey,
    forgotten,
    loadRuntimes,
    runtimesKnown,
    defaults,
    load,
    installExe,
    launchApp,
    type Bottle,
  } from "$lib/stores/windows-apps";

  onMount(() => {
    void load();
    // Separate from the bottle list: what this machine can run is a different
    // question from what is installed in it, and one failing must not blank the
    // other.
    void loadRuntimes();
  });

  // The second line under an app's name: the one thing this page is waiting
  // for if nobody has picked its program yet, otherwise the compat tier as
  // honest prose, never a "just works" promise.
  function subline(b: Bottle): string {
    if (!b.hasProgram) return $t("s.wa.programPending");
    if (b.tier === "curated") return $t("s.wa.compat.curated", { recipe: b.recipe });
    if (b.tier === "best-effort") return $t("s.wa.compat.best");
    return $t("s.wa.compat.none");
  }

  // An install runs in the installer's own window; the new bottle's page is
  // where the person picks what it left behind, so that is where they go.
  async function install() {
    const id = await installExe();
    if (id) await goto(`/windows-apps/${id}`);
  }

  const empty = $derived(!$winApps.loading && !$winApps.unavailable && $winApps.bottles.length === 0);
</script>

<Page
  title={$t("s.wa.title")}
  description={$t("s.wa.desc")}
>
  <SectionGrid>
    {#if $winApps.mocked}
      <Notice tone="neutral" class="span-full" text={$t("s.wa.mocked")} />
    {:else if $winApps.unavailable}
      <Notice tone="caution" class="span-full" text={$t("s.wa.unavailable")} />
    {/if}
    <!-- A bottle that is on disk and will not read. Named rather than dropped:
         the list below cannot hold it, so without this line it simply is not
         there and the count reads as the truth about this machine. -->
    {#if $winApps.unreadable.length > 0}
      <Notice
        tone="caution"
        class="span-full"
        text={$t("s.wa.someUnreadable", { names: $winApps.unreadable.join(", ") })}
      />
    {/if}
    {#if $forgotten}
      <Notice
        tone="neutral"
        class="span-full"
        text={$t($forgotten.trashed ? "s.wa.forgotten" : "s.wa.forgottenNoFiles", { name: $forgotten.name })}
      />
    {/if}
    {#if $winActionFailed}
      <Notice tone="error" class="span-full" text={$t("s.wa.actionFailed")} />
    {/if}
    {#if $forgetFailed}
      <Notice
        tone="error"
        class="span-full"
        text={$t(forgetFailureKey($forgetFailed.reason), { name: $forgetFailed.name })}
      />
    {/if}
    <!-- The install path's own refusal, distinct from a launch's: "nothing here
         runs Windows programs" is a different thing to do about it than "that app
         would not start". -->
    {#if $installFailed}
      <Notice tone="error" class="span-full" text={$t(installFailureKey($installFailed))} />
    {/if}
    {#if $launchFailed}
      <Notice
        tone="error"
        class="span-full"
        text={$t(launchFailureKey($launchFailed.reason), { name: $launchFailed.name })}
      />
    {/if}

    <!-- The list exists only when there is something to list. "None" is not a
         card: it is the install row below, leading. And "could not read" is the
         notice above, said once. -->
    {#if $winApps.bottles.length > 0}
      <Section label={$t("s.wa.installed")} class="span-full">
        {#each $winApps.bottles as b (b.id)}
          <!-- The whole row leads to the app's page (the /apps list pattern); the
               stretched button underneath carries the click so the Launch button
               can sit on top of it as its own control. -->
          <div class="win-row">
            <button
              type="button"
              class="win-go"
              aria-label={b.appName ?? b.id}
              onclick={() => goto(`/windows-apps/${b.id}`)}
            ></button>
            <AppAvatar appId={b.appId ?? b.id} label={b.appName ?? b.id} size={32} />
            <span class="win-text">
              <span class="win-name">{b.appName ?? b.id}</span>
              <span class="win-tier" class:pending={!b.hasProgram}>{subline(b)}</span>
            </span>
            <!-- Launch only where there is something to launch; a bottle still
                 waiting for its program has the row itself as the way to answer. -->
            {#if b.hasProgram}
              <span class="win-launch">
                <Button variant="outline" size="sm" onclick={() => launchApp(b.id)}>
                  {$t("s.wa.launchApp")}
                </Button>
              </span>
            {/if}
            <span class="chev"><ChevronRight size={15} strokeWidth={2} /></span>
          </div>
        {/each}
      </Section>
    {/if}

    {#if !$winApps.loading}
      <Section label={$t("s.wa.addApp")} class="span-full">
        <Row
          id="win-install"
          label={empty ? $t("s.wa.none") : $t("s.wa.installApp")}
          description={$t("s.wa.installAppDesc")}
        >
          {#snippet control()}
            <Button variant={empty ? "default" : "outline"} size="sm" onclick={install}>
              {$t("s.wa.chooseInstaller")}
            </Button>
          {/snippet}
        </Row>
      </Section>
    {/if}

    <!-- A reading of this machine, nothing else. It carried a default version
         and a bottle mode while they had somewhere to write; a preference drawn
         from nothing, presented next to a measurement, is the shape this panel
         was cleaned of in August. They come back with a backend that reads them. -->
    <Section label={$t("s.wa.defaults")} class="span-full">
      <!-- No runtimes rather than four invented ones: the list of what is
           installed is an observation. Empty means two different things and
           they get two different rows - the runtime was asked and there is none,
           or nobody could ask. -->
      {#if $defaults.runtimes.length === 0}
        <Row
          id="win-runtimes"
          label={$runtimesKnown ? $t("s.wa.runtimesNone") : $t("s.wa.runtimesUnknown")}
          description={$runtimesKnown
            ? $t("s.wa.runtimesNoneDesc")
            : $t("s.wa.runtimesUnknownDesc")}
        />
      {/if}
      {#each $defaults.runtimes as r (r.name)}
        <Row label={r.name}>
          {#snippet control()}
            <span class="win-installed">{$t("s.wa.runtimeInstalled")}</span>
          {/snippet}
        </Row>
      {/each}
    </Section>
  </SectionGrid>
</Page>

<style>
  /* One row per app inside the Section card (the card draws the dividers). */
  .win-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: var(--space-row, 0.75rem) 1rem;
  }
  .win-row:hover {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .win-go {
    position: absolute;
    inset: 0;
    border: none;
    background: transparent;
    cursor: pointer;
  }
  .win-text {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    flex: 1;
    min-width: 0;
  }
  .win-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .win-tier {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The one line on this page that asks for something: a shade louder than
     a tier, still not an alarm. */
  .win-tier.pending {
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }
  /* Above the stretched row button, so Launch is its own click. */
  .win-launch {
    position: relative;
    display: inline-flex;
  }
  .chev {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .win-installed {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
