<script lang="ts">
  /// The remote-session scope/audit bar (terminal.md §4.12): sits above the console
  /// for a remote session and states only the reach the broker actually enforces,
  /// never a cosmetic label. Carries the host badge + project tint + breadcrumb, the
  /// enforced reach, the audit + key-custody facts, a revoke-from-here kill, and the
  /// honest blocks-or-plain indicator. Local sessions render no bar.
  import { ShieldCheck, Radio, Layers, TerminalSquare, Loader, LogOut } from "lucide-svelte";
  import { activeRemote, revokeFromHere } from "$lib/stores/remoteConnections";
</script>

{#if $activeRemote}
  {@const r = $activeRemote}
  <div class="rsb" style={r.projectTint ? `--tint:${r.projectTint}` : "--tint:var(--color-fg-secondary)"}>
    <span class="rsb-badge"></span>
    <span class="rsb-host">{r.label}</span>
    <span class="rsb-addr">{r.user}@{r.host}</span>
    {#if r.project}<span class="rsb-proj">{r.project}</span>{/if}

    <span class="rsb-div"></span>

    <span class="rsb-reach">
      {#each r.reach as cap (cap)}<span class="rsb-chip">{cap}</span>{/each}
    </span>
    {#if r.via}<span class="rsb-fact">via {r.via}</span>{/if}
    {#if r.recorded}<span class="rsb-fact"><Radio size={12} strokeWidth={2} /> recorded</span>{/if}
    {#if r.keyInBroker}<span class="rsb-fact"><ShieldCheck size={12} strokeWidth={2} /> key never left the broker</span>{/if}

    <span class="rsb-spacer"></span>

    {#if r.bootstrap === "blocks"}
      <span class="rsb-blocks ok"><Layers size={12} strokeWidth={2} /> Blocks active</span>
    {:else if r.bootstrap === "plain"}
      <span class="rsb-blocks"><TerminalSquare size={12} strokeWidth={2} /> Plain terminal, shell integration unavailable</span>
    {:else}
      <span class="rsb-blocks"><Loader size={12} strokeWidth={2} /> Connecting…</span>
    {/if}

    <button class="rsb-revoke" onclick={() => revokeFromHere()} aria-label="Disconnect and revoke">
      <LogOut size={13} strokeWidth={2} /> Disconnect
    </button>
  </div>
{/if}

<style>
  .rsb {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    height: var(--height-bar, 36px);
    padding: 0 0.75rem;
    border-bottom: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--tint) 8%, var(--color-bg-card));
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
    overflow: hidden;
    white-space: nowrap;
  }
  .rsb-badge {
    width: 8px;
    height: 8px;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: var(--tint);
  }
  .rsb-host {
    font-weight: 600;
    color: var(--foreground);
  }
  .rsb-addr {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .rsb-proj {
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--tint) 20%, transparent);
    color: color-mix(in srgb, var(--tint) 90%, var(--foreground));
    font-weight: 500;
    font-size: 0.6875rem;
  }
  .rsb-div {
    width: 1px;
    height: 1rem;
    flex-shrink: 0;
    background: var(--color-border);
  }
  .rsb-reach {
    display: inline-flex;
    gap: 0.25rem;
  }
  .rsb-chip {
    padding: 0.05rem 0.4rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-weight: 500;
  }
  .rsb-fact {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    color: color-mix(in srgb, var(--foreground) 48%, transparent);
  }
  .rsb-spacer {
    flex: 1;
  }
  .rsb-blocks {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .rsb-blocks.ok {
    color: var(--color-success);
  }
  .rsb-revoke {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    flex-shrink: 0;
    height: var(--height-control-compact, 24px);
    padding: 0 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-error) 35%, transparent);
    border-radius: var(--radius-button);
    background: transparent;
    color: var(--color-error);
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
  }
  .rsb-revoke:hover {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
  }
</style>
