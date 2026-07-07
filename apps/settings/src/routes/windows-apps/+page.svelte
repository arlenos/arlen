<script lang="ts">
  /// Windows apps / Compatibility (windows-apps-plan.md). Windows apps run in a
  /// managed compatibility layer that is auto-configured for known apps, so this
  /// panel is mostly honest status + an escape hatch: the installed apps with their
  /// compat tier (curated-verified vs best-effort, never "just works"), an install
  /// entry, and a per-app Advanced expand for the rare tinkerer.
  import { onMount } from "svelte";
  import { ChevronDown, Trash2 } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    winApps,
    wineVersions,
    load,
    installExe,
    setWineVersion,
    deleteBottle,
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

  // The compat tier as honest prose, never a "just works" promise.
  function compatLine(b: Bottle): string {
    return b.tier === "curated"
      ? `Curated and verified, using the ${b.recipe}`
      : "Best effort on the default setup, it may not run perfectly";
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
                <div class="wa-adv-row">
                  <span class="wa-adv-label">Compatibility version</span>
                  <PopoverSelect
                    value={b.wineVersion}
                    options={versionOptions}
                    width="180px"
                    ariaLabel="Compatibility version"
                    onchange={(v) => setWineVersion(b.id, v)}
                  />
                </div>
                <div class="wa-adv-row">
                  <span class="wa-adv-label">Tweaks the recipe applied</span>
                  <span class="wa-chips">
                    {#each [...b.dllOverrides, ...b.winetricks] as t (t)}
                      <span class="wa-chip">{t}</span>
                    {/each}
                    {#if b.dllOverrides.length === 0 && b.winetricks.length === 0}
                      <span class="wa-none">None</span>
                    {/if}
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
  :global(.wa-chev) {
    transition: transform var(--duration-micro, 120ms) var(--ease-out, ease);
  }
  :global(.wa-rot) {
    transform: rotate(180deg);
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

  .wa-adv {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    padding: 0.5rem 0 0.25rem;
  }
  .wa-adv-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
  }
  .wa-adv-label {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .wa-chips {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 0.3rem;
    justify-content: flex-end;
  }
  .wa-chip {
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: 0.6875rem;
    font-family: var(--font-mono, ui-monospace, monospace);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .wa-none {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .wa-adv-foot {
    display: flex;
    justify-content: flex-end;
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
</style>
