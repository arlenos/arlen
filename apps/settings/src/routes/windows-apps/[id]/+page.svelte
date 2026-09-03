<script lang="ts">
  /// One Windows app's page (windows-apps-plan.md): identity first, then the
  /// question an install leaves behind (which program is the app), then the
  /// sovereign angle - the drive table is the bottle's file boundary made
  /// visible, one row per letter the app can see - then the compatibility,
  /// launch and tweak depth, files, removal last. A bottle whose prefix no
  /// longer matches its description says so above everything below it,
  /// because every access row under it could be understating.
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { FolderOpen, Eraser, ArrowLeft } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import AppAvatar from "$lib/components/privacy/AppAvatar.svelte";
  import {
    winApps,
    winActionFailed,
    launchFailed,
    launchFailureKey,
    forgetFailed,
    forgetFailureKey,
    fileActionFailed,
    programFailed,
    programFailureKey,
    installStarted,
    cleared,
    formatSize,
    wineVersions,
    load,
    patchBottle,
    launchApp,
    bottleHealth,
    browseFiles,
    clearCaches,
    deleteBottle,
    bottlePrograms,
    setBottleProgram,
    type BottleProgram,
    type Bottle,
    type BottleHealth,
  } from "$lib/stores/windows-apps";
  import { t } from "$lib/i18n/messages";

  const bottleId = $derived($page.params.id ?? "");
  const bottle = $derived($winApps.bottles.find((b) => b.id === bottleId));

  onMount(load);

  // The prefix-vs-description check. `null` means it could not be read, and
  // that is NOT the same as healthy: nothing renders rather than a green light.
  let health = $state<BottleHealth | null>(null);
  $effect(() => {
    if (!bottle) return;
    void bottleHealth(bottle.id).then((h) => (health = h));
  });

  // What the installer left, asked for ONLY while nobody has picked yet. A bottle
  // with a program does not need the list, and walking a prefix to build one
  // nobody will read costs the same as one somebody will. Asked again on
  // demand: nothing says when an installer is done, so the person looks.
  let programs = $state<BottleProgram[]>([]);
  let programsCut = $state(false);
  async function readPrograms(id: string) {
    const r = await bottlePrograms(id);
    programs = r.programs;
    programsCut = r.truncated;
  }
  $effect(() => {
    if (!bottle || bottle.hasProgram) {
      programs = [];
      programsCut = false;
      return;
    }
    void readPrograms(bottle.id);
  });

  const versionOptions = wineVersions.map((v) => ({ value: v, label: v }));
  const winVersionOptions = [
    { value: "7", label: "7" },
    { value: "10", label: "10" },
    { value: "11", label: "11" },
  ];
  // Derived, not constant: a top-level array is built once at import, so its
  // labels would hold whichever language was loaded then and stay in it.
  const windowModeOptions = $derived([
    { value: "windowed", label: $t("s.wa.windowed") },
    { value: "fullscreen", label: $t("s.wa.fullscreen") },
  ]);

  // The line under the name: what the page is waiting for, or the compat tier
  // as honest prose, never a "just works" promise.
  function subline(b: Bottle): string {
    if (!b.hasProgram) return $t("s.wa.programPending");
    if (b.tier === "curated") return $t("s.wa.compat.curated", { recipe: b.recipe });
    if (b.tier === "best-effort") return $t("s.wa.compat.best");
    return $t("s.wa.compat.none");
  }

  // The files row's second line: what the last clear freed, else the measured
  // size, else nothing - a size nobody measured is not a line.
  function filesLine(b: Bottle): string | undefined {
    if ($cleared?.id === b.id) {
      return $t("s.wa.clearedCaches", { size: formatSize($cleared.bytes), count: $cleared.files });
    }
    return b.diskUsage ? $t("s.wa.storageUsed", { size: b.diskUsage }) : undefined;
  }

  let confirmDelete = $state(false);
  async function onDeleteConfirmed() {
    if (!bottle) return;
    const id = bottle.id;
    confirmDelete = false;
    await deleteBottle(id);
    // The store rolls a failed delete back, so the app still being in the list
    // means the page under our feet still exists; only leave when it is gone.
    if (!$winApps.bottles.some((b) => b.id === id)) await goto("/windows-apps");
  }
</script>

<!-- The identity head is the page's hero; a Page title above it would just
     repeat the name. -->
<Page>
  <SectionGrid>
    <!-- The way back. Every other sub-page's parent sits in the sidebar; this
         panel is delisted until the daemon lands, so the page carries its own
         return, named after where it goes. -->
    <div class="back span-full">
      <Button variant="ghost" size="sm" onclick={() => goto("/windows-apps")}>
        <ArrowLeft size={15} strokeWidth={2} /> {$t("s.wa.allApps")}
      </Button>
    </div>

    {#if $winApps.mocked}
      <Notice tone="neutral" class="span-full" text={$t("s.wa.mocked")} />
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
    {#if $launchFailed}
      <Notice
        tone="error"
        class="span-full"
        text={$t(launchFailureKey($launchFailed.reason), { name: $launchFailed.name })}
      />
    {/if}
    {#if $programFailed}
      <Notice tone="error" class="span-full" text={$t(programFailureKey($programFailed))} />
    {/if}
    {#if $fileActionFailed}
      <Notice
        tone="error"
        class="span-full"
        text={$t($fileActionFailed.action === "browse" ? "s.wa.browseFailed" : "s.wa.clearFailed", {
          name: $fileActionFailed.name,
        })}
      />
    {/if}

    {#if $winApps.loading}
      <!-- Nothing yet: an id is neither found nor missing until the list
           answered, and a refusal that flashes before the answer is a lie. -->
    {:else if !bottle}
      <!-- Absent covers both an id that never existed and a list that could
           not be read; the sentence claims no more than that. -->
      <Section class="span-full">
        <p class="quiet">{$t("s.wa.notFound")}</p>
      </Section>
    {:else}
      <div class="head span-full">
        <AppAvatar appId={bottle.appId ?? bottle.id} label={bottle.appName ?? bottle.id} size={48} />
        <div class="head-text">
          <span class="head-name">{bottle.appName ?? bottle.id}</span>
          <span class="head-meta">{subline(bottle)}</span>
        </div>
        <div class="head-actions">
          <!-- Disabled, not hidden, while nothing is picked: the button is the
               page's primary action and the line beside the name says why it
               waits. -->
          <Button
            variant="default"
            size="sm"
            disabled={!bottle.hasProgram}
            onclick={() => bottle && launchApp(bottle.id)}
          >
            {$t("s.wa.launchApp")}
          </Button>
        </div>
      </div>

      <!-- An install started from this window runs in the installer's own
           window, and nothing says when it is done; the page says what is
           happening and what comes after. -->
      {#if $installStarted === bottle.id && !bottle.hasProgram}
        <Notice tone="neutral" class="span-full" text={$t("s.wa.installerRunning")} />
      {/if}

      {#if health && (!health.agrees || health.escapes > 0)}
        <Notice tone="caution" class="span-full" text={$t("s.wa.healthWarn", { count: health.escapes })} />
      {/if}

      <!-- THE QUESTION AN INSTALL LEAVES BEHIND. A Windows installer does not say
           what it installed, so between running one and starting the app there is
           a step only a person can take. Without this card the launch button
           refuses with "nothing to run" and nothing on screen says what to do
           about it. It sits above the access rows because it is the one thing
           this page is waiting for. -->
      {#if !bottle.hasProgram}
        <Section label={$t("s.wa.whichProgram")} class="span-full">
          <!-- A sentence and not a Row label: a Row truncates to one line, and
               the half of this sentence that matters is the second half. The
               look-again sits with it because the list can change under the
               page while an installer is still writing. -->
          <div class="lead">
            <p class="quiet">
              {programs.length === 0 ? $t("s.wa.whichProgramNone") : $t("s.wa.whichProgramDesc")}
            </p>
            <Button variant="ghost" size="sm" onclick={() => bottle && readPrograms(bottle.id)}>
              {$t("s.wa.lookAgain")}
            </Button>
          </div>
          {#each programs as p (p.path)}
            <Row id={`win-program-${p.name}`} label={p.name}>
              {#snippet control()}
                <Button
                  variant="outline"
                  size="sm"
                  onclick={() => bottle && setBottleProgram(bottle.id, p.path)}
                >
                  {$t("s.wa.useThis")}
                </Button>
              {/snippet}
            </Row>
          {/each}
          {#if programsCut}
            <p class="quiet">{$t("s.wa.whichProgramMore")}</p>
          {/if}
        </Section>
      {/if}

      <Section label={$t("s.wa.access")} class="span-full">
        {#each bottle.drives as d (d.letter)}
          <Row id={`win-drive-${d.letter.toLowerCase()}`} label={d.path ?? $t("s.wa.driveOwn")}>
            {#snippet leading()}
              <span class="drive-chip">{d.letter}:</span>
            {/snippet}
          </Row>
        {/each}
        <Row
          id="win-network"
          label={$t("s.wa.network")}
          description={bottle.access.network ? $t("s.wa.networkOn") : $t("s.wa.networkOff")}
        />
        <Row id="win-manage-access" label={$t("s.wa.manageReach")}>
          {#snippet control()}
            <Button variant="outline" size="sm" onclick={() => bottle && goto(`/apps/${bottle.appId}`)}>
              {$t("s.wa.manageAccess")}
            </Button>
          {/snippet}
        </Row>
      </Section>

      <!-- THE RECIPE HALF, and it is absent until there is a recipe. Every
           control below binds a value that comes from the compat recipe - Wine
           version, DLL overrides, winetricks verbs, DXVK, scaling, the window
           mode - and `windows-apps-plan.md` lists that recipe as its own piece,
           forage-distributed and signed, which does not exist yet. Drawn from an
           unmeasured value each of these renders a POSITION, and each writes
           through `set_bottle_config`, which no host defines. So the section
           says what it is rather than offering switches that go nowhere. -->
      {#if bottle.wineVersion !== undefined}
        <!-- The `?? ...` fallbacks inside are UNREACHABLE behind this gate: the
             recipe half arrives whole or not at all. They are there because the
             check narrows `wineVersion` and TypeScript cannot carry that to its
             neighbours, not because any of them is a value worth showing. -->
        <Section label={$t("s.wa.compat")} class="span-full">
          <Row id="win-wine-version" label={$t("s.wa.compatVersion")}>
            {#snippet control()}
              <PopoverSelect
                value={bottle.wineVersion ?? ""}
                options={versionOptions}
                ariaLabel={$t("s.wa.compatVersion")}
                onchange={(v) => bottle && patchBottle(bottle.id, { wineVersion: v })}
              />
            {/snippet}
          </Row>
          <Row id="win-windows-version" label={$t("s.wa.winVersion")}>
            {#snippet control()}
              <SegmentedControl
                value={bottle.windowsVersion ?? "10"}
                options={winVersionOptions}
                ariaLabel={$t("s.wa.winVersion")}
                onchange={(v) => bottle && patchBottle(bottle.id, { windowsVersion: v as Bottle["windowsVersion"] })}
              />
            {/snippet}
          </Row>
          <Row id="win-dxvk" label={$t("s.wa.dxvk")}>
            {#snippet control()}
              <Switch
                value={bottle.dxvk ?? false}
                ariaLabel={$t("s.wa.dxvkAria")}
                onchange={(v) => bottle && patchBottle(bottle.id, { dxvk: v })}
              />
            {/snippet}
          </Row>
          <Row id="win-scaling" label={$t("s.wa.scaling")}>
            {#snippet control()}
              <NumberInput
                value={bottle.scaling ?? 100}
                min={100}
                max={300}
                step={25}
                unit="%"
                ariaLabel={$t("s.wa.scaling")}
                onchange={(v) => bottle && patchBottle(bottle.id, { scaling: v })}
              />
            {/snippet}
          </Row>
          <Row id="win-window-mode" label={$t("s.wa.windowMode")}>
            {#snippet control()}
              <SegmentedControl
                value={bottle.windowMode ?? "windowed"}
                options={windowModeOptions}
                ariaLabel={$t("s.wa.windowMode")}
                onchange={(v) => bottle && patchBottle(bottle.id, { windowMode: v as Bottle["windowMode"] })}
              />
            {/snippet}
          </Row>
          <Row id="win-follow-theme" label={$t("s.wa.followTheme")}>
            {#snippet control()}
              <Switch
                value={bottle.followsTheme ?? false}
                ariaLabel={$t("s.wa.followTheme")}
                onchange={(v) => bottle && patchBottle(bottle.id, { followsTheme: v })}
              />
            {/snippet}
          </Row>
        </Section>

        <Section label={$t("s.wa.launch")} class="span-full">
          <Row id="win-launch-args" label={$t("s.wa.args")}>
            {#snippet control()}
              <Input
                value={bottle.launchArgs ?? ""}
                placeholder={$t("s.wa.argsHint")}
                aria-label={$t("s.wa.args")}
                oninput={(e) => bottle && patchBottle(bottle.id, { launchArgs: e.currentTarget.value })}
              />
            {/snippet}
          </Row>
          <Row id="win-working-dir" label={$t("s.wa.workDir")}>
            {#snippet control()}
              <Input
                value={bottle.workingDir ?? ""}
                placeholder={$t("s.wa.default")}
                aria-label={$t("s.wa.workDir")}
                oninput={(e) => bottle && patchBottle(bottle.id, { workingDir: e.currentTarget.value })}
              />
            {/snippet}
          </Row>
          <Row id="win-env" label={$t("s.wa.env")}>
            {#snippet below()}
              <div class="chips">
                <ChipList
                  items={bottle.envVars ?? []}
                  placeholder={$t("s.wa.envHint")}
                  onchange={(items) => bottle && patchBottle(bottle.id, { envVars: items })}
                />
              </div>
            {/snippet}
          </Row>
        </Section>

        <Section label={$t("s.wa.tweaks")} class="span-full">
          <Row id="win-dll" label={$t("s.wa.dll")}>
            {#snippet below()}
              <div class="chips">
                <ChipList
                  items={bottle.dllOverrides ?? []}
                  placeholder={$t("s.wa.dllHint")}
                  onchange={(items) => bottle && patchBottle(bottle.id, { dllOverrides: items })}
                />
              </div>
            {/snippet}
          </Row>
          <Row id="win-winetricks" label={$t("s.wa.winetricks")}>
            {#snippet below()}
              <div class="chips">
                <ChipList
                  items={bottle.winetricks ?? []}
                  placeholder={$t("s.wa.winetricksHint")}
                  onchange={(items) => bottle && patchBottle(bottle.id, { winetricks: items })}
                />
              </div>
            {/snippet}
          </Row>
        </Section>
      {:else}
        <Section label={$t("s.wa.compat")} class="span-full">
          <p class="quiet">{$t("s.wa.notManaged")}</p>
        </Section>
      {/if}

      <Section label={$t("s.wa.files")} class="span-full">
        <Row id="win-storage" label={$t("s.wa.filesLabel")} description={filesLine(bottle)}>
          {#snippet control()}
            <span class="file-btns">
              <Button variant="outline" size="sm" onclick={() => bottle && browseFiles(bottle.id)}>
                <FolderOpen size={14} strokeWidth={2} /> {$t("s.wa.browse")}
              </Button>
              <Button variant="ghost" size="sm" onclick={() => bottle && clearCaches(bottle.id)}>
                <Eraser size={14} strokeWidth={2} /> {$t("s.wa.clearCaches")}
              </Button>
            </span>
          {/snippet}
        </Row>
      </Section>

      <!-- Removal is not a file operation, so it does not sit under the Files
           label; its own card keeps the destructive edge apart. -->
      <Section class="span-full">
        <Row id="win-delete" label={$t("s.wa.deleteApp")} description={$t("s.wa.deleteDesc")}>
          {#snippet control()}
            <Button variant="destructive" size="sm" onclick={() => (confirmDelete = true)}>
              {$t("s.wa.confirmLabel")}
            </Button>
          {/snippet}
        </Row>
      </Section>
    {/if}
  </SectionGrid>
</Page>

<ConfirmDialog
  open={confirmDelete}
  title={$t("s.wa.confirmTitle")}
  message={$t("s.wa.confirmMsg", { name: bottle?.appName ?? bottle?.id ?? "" })}
  confirmLabel={$t("s.wa.confirmLabel")}
  variant="destructive"
  onConfirm={onDeleteConfirmed}
  onCancel={() => (confirmDelete = false)}
/>

<style>
  /* The ghost button carries its own padding; pulled in so its label sits on
     the grid edge with the sections below. */
  .back {
    margin-inline-start: -0.65rem;
  }

  /* A sentence inside a Section card, on the row inset: the register for an
     empty list, a gate that says what it is, a note under a list. */
  .quiet {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  /* The sentence with its one action beside it, as a Row lays a control. */
  .lead {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding-inline-end: 0.75rem;
  }
  .lead .quiet {
    flex: 1;
    min-width: 0;
  }

  /* Identity head: the app's mark and name anchor the page; the launch sits
     pinned to the right like a desktop detail view, not an inline link. */
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

  /* A drive letter the way the Windows app sees it: a fixed-width mark, so
     the letters line up into the table they are. */
  .drive-chip {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 2rem;
    padding: 0.125rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs);
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .chips {
    padding: 0.25rem 0 0.5rem;
  }
  .file-btns {
    display: inline-flex;
    gap: 0.4rem;
  }
</style>
