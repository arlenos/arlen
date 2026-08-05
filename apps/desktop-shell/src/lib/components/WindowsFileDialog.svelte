<script lang="ts">
  import { t } from "$lib/i18n/messages";
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

  // The tier is a tag, not a sentence; only "untested" earns one short line
  // (the consequence the tag alone does not carry).
  // `$derived`, not `const`: a constant would capture English at import and never
  // follow a locale switch. See `check-i18n-reactivity.mjs`.
  const TIER_LABELS: Record<PendingWindowsFile["tier"], string> = $derived({
    verified: $t("sh.wf.tierVerified"),
    "should-work": $t("sh.wf.tierShouldWork"),
    untested: $t("sh.wf.tierUntested"),
  });

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
        <!-- Identity once: the title carries the name; the tags carry what a
             sentence would otherwise repeat. -->
        <div class="wf-head">
          <span class="wf-avatar">{p.appName.charAt(0)}</span>
          <h2 class="wf-title">{$t("sh.wf.open", { app: p.appName })}</h2>
        </div>
        <div class="wf-tags">
          <span class="wf-tag">{$t("sh.wf.windowsApp")}</span>
          <span class="wf-tag tier-{p.tier}">{TIER_LABELS[p.tier]}</span>
        </div>
        {#if p.tier === "untested"}
          <p class="wf-status">{$t("sh.wf.mightNotRun")}</p>
        {/if}

        <!-- The sovereign preview: the profile this open mints, one click away. -->
        <div class="wf-access">
          <button type="button" class="wf-access-head" class:open={accessOpen} onclick={() => (accessOpen = !accessOpen)}>
            <ChevronRight size={13} strokeWidth={2} />
            {$t("sh.wf.whatItCanAccess")}
          </button>
          {#if accessOpen}
            <div class="wf-access-body">
              {#each p.access as scope (scope)}
                <div class="wf-scope">
                  <span class="wf-scope-verb">{$t("sh.wf.reaches")}</span>
                  <span class="wf-scope-object">{scope}</span>
                </div>
              {/each}
              <p class="wf-access-note">{$t("sh.wf.changeableLater")}</p>
            </div>
          {/if}
        </div>

        {#if p.fetch}
          <!-- First run: the runtime fetch is a progress step in the same
               dialog, never a setup wall. -->
          <div class="wf-fetch">
            <span class="wf-fetch-label">{$t("sh.wf.getting", { runtime: p.fetch.runtime })}</span>
            <div class="wf-fetch-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(p.fetch.progress * 100)}>
              <span class="wf-fetch-fill" style={`width:${Math.round(p.fetch.progress * 100)}%`}></span>
            </div>
          </div>
          <div class="wf-foot">
            <Button variant="outline" onclick={cancel}>{$t("sh.wf.cancel")}</Button>
          </div>
        {:else}
          <div class="wf-foot">
            <Button variant="outline" onclick={cancel}>{$t("sh.wf.cancel")}</Button>
            <span class="wf-spacer"></span>
            {#if isInstaller}
              <Button variant="ghost" onclick={() => run(p.id)}>{$t("sh.wf.runOnce")}</Button>
              <Button onclick={() => install(p.id)}><Download size={14} strokeWidth={2} /> {$t("sh.wf.install")}</Button>
            {:else}
              <Button variant="ghost" onclick={() => install(p.id)}>{$t("sh.wf.install")}</Button>
              <Button onclick={() => run(p.id)}><Play size={14} strokeWidth={2} /> {$t("sh.wf.run")}</Button>
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
  .wf-head {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .wf-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    font-size: var(--text-sm);
    font-weight: 600;
    color: var(--foreground);
  }
  .wf-tags {
    display: flex;
    align-items: center;
    gap: 0.375rem;
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
