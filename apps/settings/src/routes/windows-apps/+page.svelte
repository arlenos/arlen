<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Windows apps / Compatibility (windows-apps-plan.md). Windows apps run in a
  /// managed compatibility layer that is auto-configured for known apps, so this
  /// page keeps only what a glance needs: each installed app with its honest
  /// compat tier (curated-verified vs best-effort, never "just works") and a
  /// Launch, plus the install entry and the global defaults. Everything deeper
  /// lives on the app's own page - shallow by default, deep on demand.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import {
    winApps,
    winActionFailed,
    launchFailed,
    launchFailureKey,
    forgetFailed,
    forgetFailureKey,
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


  // The compat tier as honest prose, never a "just works" promise.
  function compatLine(b: Bottle): string {
    return b.tier === "curated"
      ? $t("s.wa.compat.curated", { recipe: b.recipe })
      : $t("s.wa.compat.best");
  }
</script>

<Page
  title={$t("s.wa.title")}
  description={$t("s.wa.desc")}
>
  <SectionGrid>
    {#if $winApps.mocked}
      <p class="note span-full">
        {$t("s.wa.mocked")}
      </p>
    {:else if $winApps.unavailable}
      <p class="note span-full">
        {$t("s.wa.unavailable")}
      </p>
    {/if}
    {#if $winActionFailed}
      <p class="note span-full" role="alert">{$t("s.wa.actionFailed")}</p>
    {/if}
    <!-- A bottle that is on disk and will not read. Named rather than dropped:
         the list above cannot hold it, so without this line it simply is not
         there and the count reads as the truth about this machine. The daemon
         keeps it apart from the ones that read for the same reason. -->
    {#if $winApps.unreadable.length > 0}
      <p class="note span-full" role="alert">
        {$t("s.wa.someUnreadable", { names: $winApps.unreadable.join(", ") })}
      </p>
    {/if}
    {#if $forgetFailed}
      <p class="note span-full" role="alert">
        {$t(forgetFailureKey($forgetFailed.reason), { name: $forgetFailed.name })}
      </p>
    {/if}
    {#if $launchFailed}
      <p class="note span-full" role="alert">{$t(launchFailureKey($launchFailed.reason), { name: $launchFailed.name })}</p>
    {/if}

    <Section label={$t("s.wa.installed")} class="span-full">
      {#if $winApps.bottles.length === 0}
        <!-- "none installed" is a claim about this machine; the banner above says
             we could not read it. The label has to be where the sentence is. -->
        <p class="empty">{$winApps.unavailable ? $t("s.wa.noneUnknown") : $t("s.wa.none")}</p>
      {/if}
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
            <span class="win-tier">{compatLine(b)}</span>
          </span>
          <span class="win-launch">
            <Button variant="outline" size="sm" onclick={() => launchApp(b.id)}>
              {$t("s.wa.launchApp")}
            </Button>
          </span>
          <span class="chev"><ChevronRight size={15} strokeWidth={2} /></span>
        </div>
      {/each}
    </Section>

    <Section label={$t("s.wa.addApp")} class="span-full">
      <Row id="win-install" label={$t("s.wa.installApp")} description={$t("s.wa.installAppDesc")}>
        {#snippet control()}
          <Button variant="default" size="sm" onclick={installExe}>{$t("s.wa.chooseInstaller")}</Button>
        {/snippet}
      </Row>
    </Section>

    <!-- Named for what it holds. It was "Defaults" while it carried two settings,
         and both of those are gone until they have somewhere to write; what is left
         is a reading of this machine. -->
    <Section label={$t("s.wa.defaults")} class="span-full">
      <!-- TWO CONTROLS USED TO STAND HERE and both wrote to `set_windows_defaults`,
           which no host defines - so clicking either reverted and raised the error
           banner. The version picker was worse than inert: it opened on the literal
           string "Wine 9.0" as this machine's default, while the rows below MEASURE
           what Wine is actually here and can answer that there is none. A default
           drawn from nothing, presented next to a measurement, is the shape this
           panel was cleaned of in August.

           They come back when they have somewhere to write: a bottle mode is a real
           preference once the install path reads one, and a version picker is a real
           choice once more than one runtime can be installed. Until then the section
           says what is on the machine and nothing else. -->
      <!-- No runtimes rather than four invented ones: the list of what is
           installed is an observation. Empty now means two different things and
           they get two different rows - the runtime was asked and there is none,
           or nobody could ask. Telling somebody "none installed" when the daemon
           was simply unreachable sends them looking for the wrong problem. -->
      {#if $defaults.runtimes.length === 0}
        <Row
          id="win-runtimes"
          label={$runtimesKnown ? $t("s.wa.runtimesNone") : $t("s.wa.runtimesUnknown")}
          description={$runtimesKnown
            ? $t("s.wa.runtimesNoneDesc")
            : $t("s.wa.runtimesUnknownDesc")}
        />
      {/if}
      <!-- The state lives in the control slot alone; a description repeating
           "Installed" said the same thing twice on one line. -->
      {#each $defaults.runtimes as r (r.name)}
        <Row label={r.name}>
          {#snippet control()}
            {#if r.installed}
              <span class="win-installed">{$t("s.wa.runtimeInstalled")}</span>
            {:else}
              <Button variant="outline" size="sm">{$t("s.wa.install")}</Button>
            {/if}
          {/snippet}
        </Row>
      {/each}
    </Section>
  </SectionGrid>
</Page>

<style>
  .note {
    margin: 0;
    padding: 0 0.25rem 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

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
