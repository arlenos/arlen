<script lang="ts">
  /// The open-a-Windows-file dialog (windows-apps-plan.md §41-60): a sovereign trust
  /// moment when a .exe/.msi is opened, not a setup wall. A sibling of the consent
  /// dialog - same chrome, same calm density: identity, one question, one honest
  /// line, decide. Mounted once in +layout, inert when nothing is pending. Run and
  /// Install are reversible, so there is no hold-to-confirm.
  import { onMount } from "svelte";
  import { get } from "svelte/store";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Play, Download, ChevronRight } from "lucide-svelte";
  import {
    current,
    run,
    install,
    cancel,
    openWindowsFile,
    type PendingWindowsFile,
  } from "$lib/stores/windowsFile";

  onMount(() => {
    void openWindowsFile();
  });

  // A pending open request must always be cancellable by Escape. `open` is
  // controlled (static true; the request clears via the store), which does not
  // reliably fire the primitive's escape-close, so cancel explicitly here.
  function onWindowKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape" && get(current)) cancel();
  }

  // The tier wears its own quiet tag beside "Windows app"; the status line
  // keeps the certainty sentence, stated plainly, never "just works".
  const TIER_LABELS: Record<PendingWindowsFile["tier"], string> = {
    verified: "Verified",
    "should-work": "Should work",
    untested: "Untested",
  };
  function statusLine(p: PendingWindowsFile): string {
    if (p.tier === "verified") return `Verified compatible, using the ${p.recipe ?? "curated recipe"}.`;
    if (p.tier === "should-work") return "This should work. Runs sandboxed either way.";
    return "Untested, it might not run properly.";
  }

  // The sovereign preview: the permission profile this open would mint, shown
  // BEFORE the decision. Closed by default, one click away.
  let accessOpen = $state(false);
  $effect(() => {
    void $current;
    accessOpen = false;
  });
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if $current}
  {@const p = $current}
  {@const isInstaller = p.fileKind === "installer"}
  <Dialog.Root
    open={true}
    onOpenChange={(open) => {
      if (!open) cancel();
    }}
  >
    <Dialog.Content>
      <div class="wf">
        <div class="wf-req">
          <span class="wf-avatar">{p.appName.charAt(0)}</span>
          <span class="wf-req-name">{p.appName}</span>
          <span class="wf-tag">Windows app</span>
          <span class="wf-tag tier-{p.tier}">{TIER_LABELS[p.tier]}</span>
        </div>

        <h2 class="wf-title">Open {p.appName}?</h2>
        <p class="wf-status">{statusLine(p)}</p>

        <!-- The sovereign preview: the profile this open mints, before the
             decision. The disclosure names where it lives afterwards, so the
             grant is revisitable, not a one-shot. -->
        <div class="wf-access">
          <button type="button" class="wf-access-head" class:open={accessOpen} onclick={() => (accessOpen = !accessOpen)}>
            <ChevronRight size={13} strokeWidth={2} />
            What it can access
          </button>
          {#if accessOpen}
            <div class="wf-access-body">
              {#each p.access as scope (scope)}
                <div class="wf-scope">
                  <span class="wf-scope-verb">reaches</span>
                  <span class="wf-scope-object">{scope}</span>
                </div>
              {/each}
              <p class="wf-access-note">
                This profile is created when you {isInstaller ? "install" : "run"} it and lives in App access.
              </p>
            </div>
          {/if}
        </div>

        {#if p.fetch}
          <!-- First run: the runtime fetch is a progress step in the same
               dialog, never a setup wall. -->
          <div class="wf-fetch">
            <span class="wf-fetch-label">Getting {p.fetch.runtime} for this app</span>
            <div class="wf-fetch-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(p.fetch.progress * 100)}>
              <span class="wf-fetch-fill" style={`width:${Math.round(p.fetch.progress * 100)}%`}></span>
            </div>
          </div>
          <div class="wf-foot">
            <Button variant="outline" onclick={cancel}>Cancel</Button>
          </div>
        {:else}
          <div class="wf-foot">
            <Button variant="outline" onclick={cancel}>Cancel</Button>
            <span class="wf-spacer"></span>
            {#if isInstaller}
              <Button variant="ghost" onclick={() => run(p.id)}>Run once</Button>
              <Button onclick={() => install(p.id)}><Download size={14} strokeWidth={2} /> Install</Button>
            {:else}
              <Button variant="ghost" onclick={() => install(p.id)}>Install</Button>
              <Button onclick={() => run(p.id)}><Play size={14} strokeWidth={2} /> Run</Button>
            {/if}
          </div>
        {/if}
      </div>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .wf {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
  }
  .wf-req {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: var(--text-xs);
  }
  .wf-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--foreground);
  }
  .wf-req-name {
    font-weight: 600;
    color: var(--foreground);
  }
  .wf-tag {
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: var(--text-2xs);
    letter-spacing: 0.02em;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .wf-title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
    line-height: 1.35;
    color: var(--foreground);
  }
  .wf-status {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  /* The tier tag beside "Windows app": quiet, warning-toned only when the
     honest answer is "untested". */
  .wf-tag.tier-verified {
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .wf-tag.tier-untested {
    background: color-mix(in srgb, var(--color-warning, #ca8a04) 12%, transparent);
    color: color-mix(in srgb, var(--color-warning, #ca8a04) 92%, var(--foreground));
  }

  /* The minted-profile preview: closed by default, one click away. */
  .wf-access {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .wf-access-head {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    align-self: flex-start;
    padding: 0.125rem 0.25rem 0.125rem 0;
    border: none;
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
  }
  .wf-access-head:hover {
    color: var(--foreground);
  }
  .wf-access-head :global(svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .wf-access-head.open :global(svg) {
    transform: rotate(90deg);
  }
  .wf-access-body {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.625rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    border-radius: var(--radius-input);
  }
  .wf-scope {
    display: flex;
    align-items: baseline;
    gap: 0.375rem;
  }
  .wf-scope-verb {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .wf-scope-object {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .wf-access-note {
    margin: 0.125rem 0 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  /* First-run runtime fetch: a quiet progress step, the same dialog. */
  .wf-fetch {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }
  .wf-fetch-label {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .wf-fetch-bar {
    height: 4px;
    border-radius: 2px;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    overflow: hidden;
  }
  .wf-fetch-fill {
    display: block;
    height: 100%;
    border-radius: 2px;
    background: color-mix(in srgb, var(--color-accent, #6aa9e0) 75%, transparent);
  }

  .wf-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.375rem;
  }
  .wf-spacer {
    flex: 1;
  }
</style>
