<script lang="ts">
  /// Windows apps / Compatibility (windows-apps-plan.md). Windows apps run in a
  /// managed compatibility layer that is auto-configured for known apps, so the
  /// default view is thin: the installed apps with their honest compat tier
  /// (curated-verified vs best-effort, never "just works") + an install entry. The
  /// depth lives behind each app's Advanced expand (the sovereign angle leads) and a
  /// global Defaults section - shallow-by-default, deep-on-demand.
  import { onMount } from "svelte";
  import { ChevronDown, Trash2, FolderOpen, Eraser } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { navigateTo } from "$lib/stores/navigation";
  import {
    winApps,
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
  const windowModeOptions = [
    { value: "windowed", label: "Windowed" },
    { value: "fullscreen", label: "Fullscreen" },
  ];
  const bottleModeOptions = [
    { value: "per-app", label: "Its own bottle" },
    { value: "shared", label: "A shared bottle" },
  ];

  // The compat tier as honest prose, never a "just works" promise.
  function compatLine(b: Bottle): string {
    return b.tier === "curated"
      ? `Curated and verified, using the ${b.recipe}`
      : "Best effort on the default setup, it may not run perfectly";
  }

  // The sovereign angle: what the confined Windows app can reach, stated plainly.
  function accessLine(b: Bottle): string {
    const { network, homeFolder } = b.access;
    if (!network && !homeFolder) return "Limited access. It cannot reach your network or your home folder.";
    if (network && !homeFolder) return "Limited access. It can use the network, but not your home folder.";
    if (!network && homeFolder) return "It can reach your home folder, but not the network.";
    return "Broad access. It can reach your network and your home folder.";
  }
</script>

<Page
  title="Windows apps"
  description="Windows apps run in a managed compatibility layer. Known apps are set up for you, so you rarely need to touch anything here."
>
  <SectionGrid>
    {#if $winApps.mocked}
      <p class="note span-full">
        Showing example apps. Your installed Windows apps appear here once the
        compatibility runtime is set up.
      </p>
    {/if}

    <Group label="Installed apps" class="span-full">
      {#if $winApps.bottles.length === 0}
        <p class="empty">No Windows apps installed yet. Add one below.</p>
      {/if}
      {#each $winApps.bottles as b (b.id)}
        <Row label={b.appName} description={compatLine(b)}>
          {#snippet leading()}
            <span class="wa-avatar">{b.appName.charAt(0)}</span>
          {/snippet}
          {#snippet control()}
            <Button variant="ghost" size="sm" onclick={() => toggle(b.id)}>
              Advanced
              <ChevronDown size={13} strokeWidth={2} class={`wa-chev ${expanded.has(b.id) ? "wa-rot" : ""}`} />
            </Button>
          {/snippet}
          {#snippet below()}
            {#if expanded.has(b.id)}
              <div class="wa-adv">
                <!-- Access: the sovereign angle leads. -->
                <div class="wa-sec">Access</div>
                <p class="wa-access">{accessLine(b)}</p>
                <div class="wa-row">
                  <span class="wa-label">Follow the Arlen theme</span>
                  <Switch
                    value={b.followsTheme}
                    ariaLabel="Follow the Arlen theme"
                    onchange={(v) => patchBottle(b.id, { followsTheme: v })}
                  />
                </div>
                <div class="wa-row">
                  <span class="wa-label">Manage what this app can reach</span>
                  <Button variant="outline" size="sm" onclick={() => navigateTo("privacy", `app-${b.appId}`)}>
                    Manage access
                  </Button>
                </div>

                <!-- Compatibility. -->
                <div class="wa-sec">Compatibility</div>
                <div class="wa-row">
                  <span class="wa-label">Compatibility version</span>
                  <PopoverSelect
                    value={b.wineVersion}
                    options={versionOptions}
                    width="180px"
                    ariaLabel="Compatibility version"
                    onchange={(v) => patchBottle(b.id, { wineVersion: v })}
                  />
                </div>
                <div class="wa-row">
                  <span class="wa-label">Windows version</span>
                  <SegmentedControl
                    value={b.windowsVersion}
                    options={winVersionOptions}
                    ariaLabel="Windows version"
                    onchange={(v) => patchBottle(b.id, { windowsVersion: v as Bottle["windowsVersion"] })}
                  />
                </div>
                <div class="wa-row">
                  <span class="wa-label">Direct3D to Vulkan (DXVK)</span>
                  <Switch
                    value={b.dxvk}
                    ariaLabel="Direct3D to Vulkan"
                    onchange={(v) => patchBottle(b.id, { dxvk: v })}
                  />
                </div>
                <div class="wa-row">
                  <span class="wa-label">Display scaling</span>
                  <NumberInput
                    value={b.scaling}
                    min={100}
                    max={300}
                    step={25}
                    unit="%"
                    width="130px"
                    ariaLabel="Display scaling"
                    onchange={(v) => patchBottle(b.id, { scaling: v })}
                  />
                </div>
                <div class="wa-row">
                  <span class="wa-label">Window mode</span>
                  <SegmentedControl
                    value={b.windowMode}
                    options={windowModeOptions}
                    ariaLabel="Window mode"
                    onchange={(v) => patchBottle(b.id, { windowMode: v as Bottle["windowMode"] })}
                  />
                </div>

                <!-- Launch. -->
                <div class="wa-sec">Launch</div>
                <div class="wa-row">
                  <span class="wa-label">Arguments</span>
                  <span class="wa-input">
                    <Input
                      value={b.launchArgs}
                      placeholder="e.g. --safe-mode"
                      oninput={(e) => patchBottle(b.id, { launchArgs: e.currentTarget.value })}
                    />
                  </span>
                </div>
                <div class="wa-row">
                  <span class="wa-label">Working directory</span>
                  <span class="wa-input">
                    <Input
                      value={b.workingDir}
                      placeholder="Default"
                      oninput={(e) => patchBottle(b.id, { workingDir: e.currentTarget.value })}
                    />
                  </span>
                </div>
                <div class="wa-field">
                  <span class="wa-label">Environment variables</span>
                  <ChipList
                    items={b.envVars}
                    placeholder="KEY=value"
                    onchange={(items) => patchBottle(b.id, { envVars: items })}
                  />
                </div>

                <!-- Tweaks: editable now. -->
                <div class="wa-sec">Tweaks</div>
                <div class="wa-field">
                  <span class="wa-label">DLL overrides</span>
                  <ChipList
                    items={b.dllOverrides}
                    placeholder="Add a DLL override"
                    onchange={(items) => patchBottle(b.id, { dllOverrides: items })}
                  />
                </div>
                <div class="wa-field">
                  <span class="wa-label">Winetricks</span>
                  <ChipList
                    items={b.winetricks}
                    placeholder="Add a winetricks verb"
                    onchange={(items) => patchBottle(b.id, { winetricks: items })}
                  />
                </div>

                <!-- Files. -->
                <div class="wa-sec">Files</div>
                <div class="wa-row">
                  <span class="wa-label">Storage used, {b.diskUsage}</span>
                  <span class="wa-btns">
                    <Button variant="outline" size="sm" onclick={() => browseFiles(b.id)}>
                      <FolderOpen size={14} strokeWidth={2} /> Browse files
                    </Button>
                    <Button variant="ghost" size="sm" onclick={() => clearCaches(b.id)}>
                      <Eraser size={14} strokeWidth={2} /> Clear caches
                    </Button>
                  </span>
                </div>

                <div class="wa-adv-foot">
                  <button type="button" class="wa-delete" onclick={() => (confirmDelete = b)}>
                    <Trash2 size={14} strokeWidth={2} /> Delete this app
                  </button>
                </div>
              </div>
            {/if}
          {/snippet}
        </Row>
      {/each}
    </Group>

    <Group label="Add an app" class="span-full">
      <Row label="Install a Windows app" description="Pick a Windows installer, like a .exe or .msi, and the compatibility layer sets it up.">
        {#snippet control()}
          <Button variant="default" size="sm" onclick={installExe}>Choose an installer</Button>
        {/snippet}
      </Row>
    </Group>

    <Group label="Defaults" class="span-full">
      <Row label="Default compatibility version" description="New apps start on this Wine or Proton version.">
        {#snippet control()}
          <PopoverSelect
            value={$defaults.version}
            options={versionOptions}
            width="180px"
            ariaLabel="Default compatibility version"
            onchange={(v) => patchDefaults({ version: v })}
          />
        {/snippet}
      </Row>
      <Row label="New apps get" description="A private bottle keeps each app isolated. A shared one saves disk space.">
        {#snippet control()}
          <SegmentedControl
            value={$defaults.bottleMode}
            options={bottleModeOptions}
            ariaLabel="New apps get"
            onchange={(v) => patchDefaults({ bottleMode: v as "per-app" | "shared" })}
          />
        {/snippet}
      </Row>
      {#each $defaults.runtimes as r (r.name)}
        <Row label={r.name} description={r.installed ? "Installed" : "Available to install"}>
          {#snippet control()}
            {#if r.installed}
              <span class="wa-installed">Installed</span>
            {:else}
              <Button variant="outline" size="sm">Install</Button>
            {/if}
          {/snippet}
        </Row>
      {/each}
    </Group>
  </SectionGrid>
</Page>

<ConfirmDialog
  open={confirmDelete !== null}
  title="Delete this app?"
  message={`"${confirmDelete?.appName ?? ""}" and its files will be removed from this machine. This cannot be undone.`}
  confirmLabel="Delete"
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
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
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
    font-size: 0.8125rem;
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
    gap: 0.55rem;
    padding: 0.5rem 0 0.25rem;
  }
  /* A quiet sub-section header inside the Advanced expand. */
  .wa-sec {
    margin-top: 0.4rem;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .wa-sec:first-child {
    margin-top: 0;
  }
  .wa-access {
    margin: 0;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
  }
  .wa-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    min-height: 1.75rem;
  }
  .wa-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .wa-label {
    font-size: 0.8125rem;
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
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--color-error);
    cursor: pointer;
  }
  .wa-delete:hover {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
  }
  .wa-installed {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
</style>
