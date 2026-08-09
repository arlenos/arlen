<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Windows apps / Compatibility (windows-apps-plan.md). Windows apps run in a
  /// managed compatibility layer that is auto-configured for known apps, so the
  /// default view is thin: the installed apps with their honest compat tier
  /// (curated-verified vs best-effort, never "just works") + an install entry. The
  /// depth lives behind each app's Advanced expand (the sovereign angle leads) and a
  /// global Defaults section - shallow-by-default, deep-on-demand.
  import { onMount } from "svelte";
  import { ChevronDown, ChevronRight, Trash2, FolderOpen, Eraser } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { navigateTo } from "$lib/stores/navigation";
  import {
    winApps,
    winActionFailed,
    defaults,
    wineVersions,
    load,
    patchBottle,
    installExe,
    browseFiles,
    clearCaches,
    deleteBottle,
    patchDefaults,
    type Bottle,
  } from "$lib/stores/windows-apps";

  onMount(load);

  let expanded = $state<Set<string>>(new Set());
  function toggle(id: string) {
    const next = new Set(expanded);
    next.has(id) ? next.delete(id) : next.add(id);
    expanded = next;
  }

  let confirmDelete = $state<Bottle | null>(null);

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
  const bottleModeOptions = $derived([
    { value: "per-app", label: $t("s.wa.ownBottle") },
    { value: "shared", label: $t("s.wa.sharedBottle") },
  ]);

  // The compat tier as honest prose, never a "just works" promise.
  //
  // Whole sentences per case rather than a stem plus a clause: the access lines
  // differ in more than one word between languages, and German puts the negation
  // somewhere English does not.
  function compatLine(b: Bottle): string {
    return b.tier === "curated"
      ? $t("s.wa.compat.curated", { recipe: b.recipe })
      : $t("s.wa.compat.best");
  }

  // The sovereign angle: what the confined Windows app can reach, stated plainly.
  function accessLine(b: Bottle): string {
    const { network, homeFolder } = b.access;
    if (!network && !homeFolder) return $t("s.wa.access.neither");
    if (network && !homeFolder) return $t("s.wa.access.network");
    if (!network && homeFolder) return $t("s.wa.access.home");
    return $t("s.wa.access.both");
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
    <!-- The switches below are back on what the bottle really holds. -->
    {#if $winActionFailed}
      <p class="note span-full" role="alert">{$t("s.wa.actionFailed")}</p>
    {/if}

    <Section label={$t("s.wa.installed")} class="span-full">
      {#if $winApps.bottles.length === 0}
        <!-- "none installed" is a claim about this machine; the banner above says
             we could not read it. The label has to be where the sentence is. -->
        <p class="empty">{$winApps.unavailable ? $t("s.wa.noneUnknown") : $t("s.wa.none")}</p>
      {/if}
      {#each $winApps.bottles as b (b.id)}
        <Row label={b.appName} description={compatLine(b)}>
          {#snippet leading()}
            <span class="wa-avatar">{b.appName.charAt(0)}</span>
          {/snippet}
          {#snippet control()}
            <Button variant="ghost" size="sm" onclick={() => toggle(b.id)}>
              {$t("s.wa.advanced")}
              <ChevronDown size={13} strokeWidth={2} class={`wa-chev ${expanded.has(b.id) ? "wa-rot" : ""}`} />
            </Button>
          {/snippet}
          {#snippet below()}
            {#if expanded.has(b.id)}
              <div class="wa-adv">
                <!-- Access: the sovereign angle leads, so it opens by default. -->
                <Collapsible class="wa-col" open={true}>
                  <CollapsibleTrigger class="wa-sec-trigger">
                    <ChevronRight size={14} strokeWidth={2} />
                    {$t("s.wa.access")}
                  </CollapsibleTrigger>
                  <CollapsibleContent class="wa-sec-content">
                    <p class="wa-access">{accessLine(b)}</p>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.followTheme")}</span>
                      <Switch
                        value={b.followsTheme}
                        ariaLabel={$t("s.wa.followTheme")}
                        onchange={(v) => patchBottle(b.id, { followsTheme: v })}
                      />
                    </div>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.manageReach")}</span>
                      <Button variant="outline" size="sm" onclick={() => navigateTo("privacy", `app-${b.appId}`)}>
                        {$t("s.wa.manageAccess")}
                      </Button>
                    </div>
                  </CollapsibleContent>
                </Collapsible>

                <Collapsible class="wa-col">
                  <CollapsibleTrigger class="wa-sec-trigger">
                    <ChevronRight size={14} strokeWidth={2} />
                    {$t("s.wa.compat")}
                  </CollapsibleTrigger>
                  <CollapsibleContent class="wa-sec-content">
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.compatVersion")}</span>
                      <PopoverSelect
                        value={b.wineVersion}
                        options={versionOptions}
                        ariaLabel={$t("s.wa.compatVersion")}
                        onchange={(v) => patchBottle(b.id, { wineVersion: v })}
                      />
                    </div>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.winVersion")}</span>
                      <SegmentedControl
                        value={b.windowsVersion}
                        options={winVersionOptions}
                        ariaLabel={$t("s.wa.winVersion")}
                        onchange={(v) => patchBottle(b.id, { windowsVersion: v as Bottle["windowsVersion"] })}
                      />
                    </div>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.dxvk")}</span>
                      <Switch
                        value={b.dxvk}
                        ariaLabel={$t("s.wa.dxvkAria")}
                        onchange={(v) => patchBottle(b.id, { dxvk: v })}
                      />
                    </div>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.scaling")}</span>
                      <NumberInput
                        value={b.scaling}
                        min={100}
                        max={300}
                        step={25}
                        unit="%"
                        ariaLabel={$t("s.wa.scaling")}
                        onchange={(v) => patchBottle(b.id, { scaling: v })}
                      />
                    </div>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.windowMode")}</span>
                      <SegmentedControl
                        value={b.windowMode}
                        options={windowModeOptions}
                        ariaLabel={$t("s.wa.windowMode")}
                        onchange={(v) => patchBottle(b.id, { windowMode: v as Bottle["windowMode"] })}
                      />
                    </div>
                  </CollapsibleContent>
                </Collapsible>

                <Collapsible class="wa-col">
                  <CollapsibleTrigger class="wa-sec-trigger">
                    <ChevronRight size={14} strokeWidth={2} />
                    {$t("s.wa.launch")}
                  </CollapsibleTrigger>
                  <CollapsibleContent class="wa-sec-content">
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.args")}</span>
                      <span class="wa-input">
                        <Input
                          value={b.launchArgs}
                          placeholder={$t("s.wa.argsHint")}
                          oninput={(e) => patchBottle(b.id, { launchArgs: e.currentTarget.value })}
                        />
                      </span>
                    </div>
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.workDir")}</span>
                      <span class="wa-input">
                        <Input
                          value={b.workingDir}
                          placeholder={$t("s.wa.default")}
                          oninput={(e) => patchBottle(b.id, { workingDir: e.currentTarget.value })}
                        />
                      </span>
                    </div>
                    <div class="wa-field">
                      <span class="wa-label">{$t("s.wa.env")}</span>
                      <ChipList
                        items={b.envVars}
                        placeholder={$t("s.wa.envHint")}
                        onchange={(items) => patchBottle(b.id, { envVars: items })}
                      />
                    </div>
                  </CollapsibleContent>
                </Collapsible>

                <Collapsible class="wa-col">
                  <CollapsibleTrigger class="wa-sec-trigger">
                    <ChevronRight size={14} strokeWidth={2} />
                    {$t("s.wa.tweaks")}
                  </CollapsibleTrigger>
                  <CollapsibleContent class="wa-sec-content">
                    <div class="wa-field">
                      <span class="wa-label">{$t("s.wa.dll")}</span>
                      <ChipList
                        items={b.dllOverrides}
                        placeholder={$t("s.wa.dllHint")}
                        onchange={(items) => patchBottle(b.id, { dllOverrides: items })}
                      />
                    </div>
                    <div class="wa-field">
                      <span class="wa-label">{$t("s.wa.winetricks")}</span>
                      <ChipList
                        items={b.winetricks}
                        placeholder={$t("s.wa.winetricksHint")}
                        onchange={(items) => patchBottle(b.id, { winetricks: items })}
                      />
                    </div>
                  </CollapsibleContent>
                </Collapsible>

                <Collapsible class="wa-col">
                  <CollapsibleTrigger class="wa-sec-trigger">
                    <ChevronRight size={14} strokeWidth={2} />
                    {$t("s.wa.files")}
                  </CollapsibleTrigger>
                  <CollapsibleContent class="wa-sec-content">
                    <div class="wa-row">
                      <span class="wa-label">{$t("s.wa.storageUsed", { size: b.diskUsage })}</span>
                      <span class="wa-btns">
                        <Button variant="outline" size="sm" onclick={() => browseFiles(b.id)}>
                          <FolderOpen size={14} strokeWidth={2} /> {$t("s.wa.browse")}
                        </Button>
                        <Button variant="ghost" size="sm" onclick={() => clearCaches(b.id)}>
                          <Eraser size={14} strokeWidth={2} /> {$t("s.wa.clearCaches")}
                        </Button>
                      </span>
                    </div>
                  </CollapsibleContent>
                </Collapsible>

                <div class="wa-adv-foot">
                  <button type="button" class="wa-delete" onclick={() => (confirmDelete = b)}>
                    <Trash2 size={14} strokeWidth={2} /> {$t("s.wa.deleteApp")}
                  </button>
                </div>
              </div>
            {/if}
          {/snippet}
        </Row>
      {/each}
    </Section>

    <Section label={$t("s.wa.addApp")} class="span-full">
      <Row label={$t("s.wa.installApp")} description={$t("s.wa.installAppDesc")}>
        {#snippet control()}
          <Button variant="default" size="sm" onclick={installExe}>{$t("s.wa.chooseInstaller")}</Button>
        {/snippet}
      </Row>
    </Section>

    <Section label={$t("s.wa.defaults")} class="span-full">
      <Row label={$t("s.wa.defaultVersion")} description={$t("s.wa.defaultVersionDesc")}>
        {#snippet control()}
          <PopoverSelect
            value={$defaults.version}
            options={versionOptions}
            ariaLabel={$t("s.wa.defaultCompat")}
            onchange={(v) => patchDefaults({ version: v })}
          />
        {/snippet}
      </Row>
      <Row label={$t("s.wa.newAppsGet")} description={$t("s.wa.newAppsGetDesc")}>
        {#snippet control()}
          <SegmentedControl
            value={$defaults.bottleMode}
            options={bottleModeOptions}
            ariaLabel={$t("s.wa.newAppsGet")}
            onchange={(v) => patchDefaults({ bottleMode: v as "per-app" | "shared" })}
          />
        {/snippet}
      </Row>
      <!-- No runtimes rather than four invented ones: the list of what is
           installed is an observation, and nothing reports it yet. -->
      {#if $defaults.runtimes.length === 0}
        <Row label={$t("s.wa.runtimesUnknown")} description={$t("s.wa.runtimesUnknownDesc")} />
      {/if}
      {#each $defaults.runtimes as r (r.name)}
        <Row label={r.name} description={r.installed ? $t("s.wa.runtimeInstalled") : $t("s.wa.runtimeAvailable")}>
          {#snippet control()}
            {#if r.installed}
              <span class="wa-installed">{$t("s.wa.runtimeInstalled")}</span>
            {:else}
              <Button variant="outline" size="sm">{$t("s.wa.install")}</Button>
            {/if}
          {/snippet}
        </Row>
      {/each}
    </Section>
  </SectionGrid>
</Page>

<ConfirmDialog
  open={confirmDelete !== null}
  title={$t("s.wa.confirmTitle")}
  message={$t("s.wa.confirmMsg", { name: confirmDelete?.appName ?? "" })}
  confirmLabel={$t("s.wa.confirmLabel")}
  variant="destructive"
  onConfirm={async () => {
    if (confirmDelete) await deleteBottle(confirmDelete.id);
    confirmDelete = null;
  }}
  onCancel={() => (confirmDelete = null)}
/>

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
  .wa-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--foreground);
  }

  :global(.wa-chev) {
    transition: transform var(--duration-micro, 120ms) var(--ease-out, ease);
  }
  :global(.wa-rot) {
    transform: rotate(180deg);
  }

  .wa-adv {
    display: flex;
    flex-direction: column;
    gap: 0.1rem;
    padding: 0.35rem 0 0.25rem;
  }
  /* Each Advanced sub-section is a collapsible; a hairline separates them. */
  :global(.wa-col) {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  :global(.wa-col:first-child) {
    border-top: none;
  }
  :global(.wa-sec-trigger) {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: var(--text-sm);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    text-align: start;
    cursor: pointer;
  }
  :global(.wa-sec-trigger:hover) {
    color: var(--foreground);
  }
  :global(.wa-sec-trigger svg) {
    flex-shrink: 0;
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.wa-sec-trigger[data-state="open"] svg) {
    transform: rotate(90deg);
  }
  :global(.wa-sec-content) {
    display: flex;
    flex-direction: column;
    gap: 0.55rem;
    padding: 0.15rem 0.25rem 0.7rem 1.15rem;
  }
  .wa-access {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }
  .wa-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    min-height: 1.75rem;
    /* Same row-control register as the kit Row, so the bottle's field
       controls align with each other and the page above. */
    --control-width: var(--width-row-control, 200px);
  }
  .wa-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .wa-label {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .wa-input {
    width: min(280px, 60%);
  }
  .wa-btns {
    display: inline-flex;
    gap: 0.4rem;
  }
  .wa-adv-foot {
    display: flex;
    justify-content: flex-end;
    margin-top: 0.3rem;
  }
  .wa-delete {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.25rem 0.5rem;
    border: none;
    background: transparent;
    border-radius: var(--radius-input);
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--color-error);
    cursor: pointer;
  }
  .wa-delete:hover {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
  }
  .wa-installed {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
