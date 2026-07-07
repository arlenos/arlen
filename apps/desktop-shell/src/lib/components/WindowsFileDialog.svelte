<script lang="ts">
  /// The open-a-Windows-file dialog (windows-apps-plan.md §41-60): a sovereign trust
  /// moment when a .exe/.msi is opened, not a setup wall. A sibling of the consent
  /// dialog - it reuses that chrome so the request family stays one language.
  /// Mounted once in +layout, inert when nothing is pending. It identifies the app,
  /// states the compat tier honestly, makes the sandbox + the minted profile legible,
  /// and offers Run vs Install (defaulted by file type). Run/Install are reversible,
  /// so there is no hold-to-confirm here.
  import { onMount } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { ScopeChip } from "@arlen/ui-kit/components/ui/scope-chip";
  import { Play, Download } from "lucide-svelte";
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

  // The compat tier as honest prose, never a "just works" promise.
  function tierLine(p: PendingWindowsFile): string {
    if (p.tier === "verified") {
      return p.recipe ? `Verified compatible, using the ${p.recipe}.` : "Verified compatible with Arlen.";
    }
    if (p.tier === "should-work") return "This should work.";
    return "Untested. It might not run properly.";
  }
</script>

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
          <span class="wf-req-id">{p.fileName}</span>
        </div>

        <h2 class="wf-title">Open {p.appName}?</h2>
        <p class="wf-sub">This is a Windows app. Here is what happens.</p>

        <p class="wf-tier">{tierLine(p)}</p>

        <div class="wf-sandbox">
          <p class="wf-sandbox-line">It runs in a sandbox and starts with limited access:</p>
          <div class="wf-scopes">
            {#each p.access as scope (scope)}
              <ScopeChip label={scope} />
            {/each}
          </div>
        </div>

        {#if p.needsRuntime}
          <p class="wf-note">The first time, Arlen sets up {p.needsRuntime}. This can take a moment.</p>
        {/if}

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
    font-size: 0.75rem;
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
    font-size: 0.75rem;
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
    font-size: 0.625rem;
    letter-spacing: 0.02em;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .wf-req-id {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wf-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    line-height: 1.35;
    color: var(--foreground);
  }
  .wf-sub {
    margin: -0.375rem 0 0;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .wf-tier {
    margin: 0;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .wf-sandbox {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .wf-sandbox-line {
    margin: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .wf-scopes {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }
  .wf-note {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
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
