<script lang="ts">
  /// The open-a-Windows-file dialog (windows-apps-plan.md §41-60): a sovereign trust
  /// moment when a .exe/.msi is opened, not a setup wall. A sibling of the consent
  /// dialog - same chrome, same calm density: identity, one question, one honest
  /// line, decide. Mounted once in +layout, inert when nothing is pending. Run and
  /// Install are reversible, so there is no hold-to-confirm.
  import { onMount } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
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

  // The honest tier + the sandbox reach, folded into one quiet line. Never "just
  // works": each tier states its certainty plainly.
  function statusLine(p: PendingWindowsFile): string {
    const tier =
      p.tier === "verified"
        ? "Verified compatible."
        : p.tier === "should-work"
          ? "This should work."
          : "Untested, it might not run properly.";
    const reach = p.access.some((a) => /network/i.test(a))
      ? "its files and the network"
      : "its own files";
    return `${tier} Runs sandboxed with access to ${reach}.`;
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
        </div>

        <h2 class="wf-title">Open {p.appName}?</h2>
        <p class="wf-status">{statusLine(p)}</p>

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
  .wf-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    line-height: 1.35;
    color: var(--foreground);
  }
  .wf-status {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
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
