<script lang="ts">
  /// The remote-session header pill (terminal.md §4.12): folded into the window
  /// header, not a second bar. The pill states the host + the honest blocks-or-plain
  /// status; a click opens a popover with the enforced scope (reach, jump host,
  /// audit, key custody) and the revoke-from-here Disconnect. Monochrome + flat: the
  /// house has no accent colour, so identity is the label, not a tint. Local
  /// sessions render nothing.
  import * as Popover from "@arlen/ui-kit/components/ui/popover";
  import { ShieldCheck, Radio, ChevronDown, LogOut } from "lucide-svelte";
  import { activeRemote, revokeFromHere } from "$lib/stores/remoteConnections";

  let open = $state(false);

  function disconnect() {
    open = false;
    revokeFromHere();
  }
</script>

{#if $activeRemote}
  {@const r = $activeRemote}
  <Popover.Root bind:open>
    <Popover.Trigger class="rsp-pill">
      <span class="rsp-badge"></span>
      <span class="rsp-host">{r.label}</span>
      {#if r.bootstrap === "blocks"}
        <span class="rsp-status ok">Blocks</span>
      {:else if r.bootstrap === "plain"}
        <span class="rsp-status">Plain</span>
      {:else}
        <span class="rsp-status">Connecting</span>
      {/if}
      <ChevronDown class="rsp-caret" size={13} strokeWidth={2} />
    </Popover.Trigger>
    <Popover.Content align="start" sideOffset={6} class="rsp-pop">
      <div class="rsp-addr">{r.user}@{r.host}</div>
      {#if r.project}<div class="rsp-sub">{r.project}</div>{/if}

      <div class="rsp-facts">
        <div class="rsp-fact"><span class="rsp-key">reach</span> {r.reach.join(", ")}</div>
        {#if r.via}<div class="rsp-fact"><span class="rsp-key">via</span> {r.via}</div>{/if}
        {#if r.recorded}<div class="rsp-fact"><Radio size={12} strokeWidth={2} /> recorded to your timeline</div>{/if}
        {#if r.keyInBroker}<div class="rsp-fact"><ShieldCheck size={12} strokeWidth={2} /> the terminal never sees your key</div>{/if}
      </div>

      {#if r.bootstrap === "plain"}
        <div class="rsp-note">Plain terminal. This host does not support the block shell integration.</div>
      {/if}

      <button class="rsp-revoke" onclick={disconnect}>
        <LogOut size={13} strokeWidth={2} /> Disconnect
      </button>
    </Popover.Content>
  </Popover.Root>
{/if}

<style>
  /* The header pill: identity + the honest status, at the house control radius. */
  :global(.rsp-pill) {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    height: var(--height-control-compact, 26px);
    padding: 0 0.5rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-size: 0.75rem;
  }
  :global(.rsp-pill:hover) {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .rsp-badge {
    width: 8px;
    height: 8px;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .rsp-host {
    font-weight: 600;
    color: var(--foreground);
  }
  .rsp-status {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .rsp-status.ok {
    color: var(--color-success);
  }
  :global(.rsp-caret) {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  /* The scope popover: flat, plain text - the verbose detail off the header. */
  :global(.rsp-pop) {
    width: 17rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .rsp-addr {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rsp-sub {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rsp-facts {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin-top: 0.25rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .rsp-fact {
    display: flex;
    align-items: center;
    gap: 0.35rem;
  }
  .rsp-key {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .rsp-note {
    margin-top: 0.15rem;
    font-size: 0.6875rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .rsp-revoke {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.3rem;
    height: var(--height-control, 28px);
    margin-top: 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-error) 35%, transparent);
    border-radius: var(--radius-input);
    background: transparent;
    color: var(--color-error);
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
  }
  .rsp-revoke:hover {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
  }
</style>
