<script lang="ts">
  /// App access - the system-wide capability browser (living-capability-graph.md
  /// §6). Shows every principal (the assistant and each app) and what it can
  /// reach, and lets the user NARROW that reach. This is the home of revoke:
  /// the AI-scoped harness surface shows the assistant's grants read-only and
  /// defers revoking to here. Grants come from the daemon's `access_grants`
  /// projection, scoped whole-system for the `settings` principal; revoke maps
  /// to the profile-first, narrowing-only 0x06 op. The Settings Tauri bridge is
  /// unwired for now, so the store reads a fixture until it lands.
  ///
  /// Copy law: no em-dashes, no middot separators; usage is "not measured yet",
  /// never a fabricated "never".
  import { onMount } from "svelte";
  import { Sparkles, AppWindow } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { ScopeChip } from "@arlen/ui-kit/components/ui/scope-chip";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    grants,
    grantsLoaded,
    grantsError,
    groupPrincipals,
    reachLabel,
    loadGrants,
    revokeReach,
    revokeAll,
    type Principal,
  } from "$lib/stores/grants";

  onMount(loadGrants);

  const principals = $derived(groupPrincipals($grants));
  const assistants = $derived(principals.filter((p) => p.assistant));
  const apps = $derived(principals.filter((p) => !p.assistant));

  // The pending revoke, awaiting confirm. `one` narrows a single reach; `all`
  // narrows everything the principal holds.
  type Pending =
    | { kind: "one"; appId: string; label: string; reach: string }
    | { kind: "all"; appId: string; label: string; reach: string[] }
    | null;
  let pending = $state<Pending>(null);

  function askOne(p: Principal, reach: string) {
    pending = { kind: "one", appId: p.appId, label: p.label, reach };
  }
  function askAll(p: Principal) {
    pending = { kind: "all", appId: p.appId, label: p.label, reach: [...p.reach] };
  }

  const confirmTitle = $derived(
    pending?.kind === "all" ? "Remove all access?" : "Remove access?",
  );
  const confirmMessage = $derived(
    pending === null
      ? ""
      : pending.kind === "all"
        ? `${pending.label} will no longer reach anything on your system. It can ask again if it needs to.`
        : `${pending.label} will no longer reach your ${reachLabel(pending.reach)}. It can ask again if it needs to.`,
  );
  const confirmLabel = $derived(
    pending?.kind === "all" ? "Remove all" : "Remove",
  );

  async function onConfirm() {
    if (pending === null) return;
    if (pending.kind === "all") await revokeAll(pending.appId, pending.reach);
    else await revokeReach(pending.appId, pending.reach);
    pending = null;
  }
</script>

<Page
  title="App access"
  description="See what each app and the assistant can reach on your system. You can remove access here. You cannot grant it; that happens when an app asks and you agree."
>
  <SectionGrid>
    {#if $grantsError}
      <Group label="App access" class="span-full">
        <p class="note">Could not read app access. The permission service did not answer.</p>
      </Group>
    {:else if $grantsLoaded && principals.length === 0}
      <Group label="App access" class="span-full">
        <p class="note">No app holds access to your data.</p>
      </Group>
    {:else}
      {#if assistants.length > 0}
        <Group label="The assistant" class="span-full">
          {#each assistants as p (p.appId)}
            {@render grantRow(p)}
          {/each}
        </Group>
      {/if}

      {#if apps.length > 0}
        <Group label="Apps" class="span-full">
          {#each apps as p (p.appId)}
            {@render grantRow(p)}
          {/each}
        </Group>
      {/if}

      {#if $grantsLoaded && principals.length > 0}
        <p class="usage-note span-full">
          Usage is not measured yet, so this shows what each app can reach, not
          what it has used.
        </p>
      {/if}
    {/if}
  </SectionGrid>
</Page>

{#snippet grantRow(p: Principal)}
  <Row
    label={p.label}
    description={p.identityVerified ? undefined : "Identity not verified."}
    id={`grant-${p.appId}`}
  >
    {#snippet leading()}
      {#if p.assistant}
        <Sparkles size={18} strokeWidth={1.5} class="lead-icon" />
      {:else}
        <AppWindow size={18} strokeWidth={1.5} class="lead-icon" />
      {/if}
    {/snippet}
    {#snippet control()}
      <Button variant="ghost" size="sm" onclick={() => askAll(p)}>
        Remove all
      </Button>
    {/snippet}
    {#snippet below()}
      <div class="reach">
        {#each p.reach as r (r)}
          <ScopeChip label={reachLabel(r)} onRevoke={() => askOne(p, r)} />
        {/each}
      </div>
    {/snippet}
  </Row>
{/snippet}

<ConfirmDialog
  open={pending !== null}
  title={confirmTitle}
  message={confirmMessage}
  confirmLabel={confirmLabel}
  variant="destructive"
  {onConfirm}
  onCancel={() => (pending = null)}
/>

<style>
  .reach {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    /* Align the chips under the label, past the Row's leading icon column
       (icon 1.125rem + the row-main gap 0.875rem). */
    padding-left: 2rem;
  }
  .note {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .usage-note {
    margin: 0;
    padding: 0 0.25rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  :global(.lead-icon) {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
