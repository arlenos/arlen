<script lang="ts">
  /// The duplicate finder in place of the listing: byte-identical files grouped
  /// by content, with a keep/trash control per copy. The safety floor is visible
  /// here: exactly one copy per group stays kept (the last kept one cannot be
  /// marked), the action is trash via a confirm, and the default keeps the
  /// newest. The scan itself is the backend's (`scanDuplicates`); this reviews.
  import { Loader2, Lock, Trash2 } from "lucide-svelte";
  import { entryIcon, formatSize, formatModified } from "@arlen/ui-kit/components/browser";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { t } from "$lib/i18n/messages";
  import {
    duplicateGroups,
    duplicatesScanning,
    duplicatesScope,
    trashMarks,
    markedPaths,
    reclaimable,
    groupReclaimable,
    keptCount,
    keepNewest,
    toggleTrash,
    closeDuplicates,
    type DupFile,
    type DupGroup,
  } from "$lib/stores/duplicates";

  let {
    ontrash,
  }: {
    /// Move the chosen copies to the trash (the page runs the op); called only
    /// after the confirm.
    ontrash?: (paths: string[]) => void;
  } = $props();

  const groups = $derived($duplicateGroups);
  const shortScope = $derived($duplicatesScope.replace(/^\/home\/[^/]+/, "~"));
  const dirOf = (p: string) => p.slice(0, p.lastIndexOf("/")) || "/";

  let confirming = $state(false);

  function entryOf(f: DupFile) {
    return { name: f.name, kind: "file" as const };
  }
</script>

<div class="dup">
  {#if $duplicatesScanning}
    <div class="dup-state">
      <Loader2 class="spin" size={20} strokeWidth={2} />
      <span class="dup-state-title">{$t("f.dup.scanning", { scope: shortScope })}</span>
      <span class="dup-state-hint">{$t("f.dup.scanningHint")}</span>
    </div>
  {:else if groups === null}
    <div class="dup-state">
      <span class="dup-state-title">{$t("f.action.findDuplicates")}</span>
      <span class="dup-state-hint">{$t("f.dup.findHint", { scope: shortScope })}</span>
    </div>
  {:else if groups.length === 0}
    <div class="dup-state">
      <span class="dup-state-title">{$t("f.dup.none", { scope: shortScope })}</span>
      <span class="dup-state-hint">{$t("f.dup.noneHint")}</span>
    </div>
  {:else}
    <div class="dup-head">
      <div class="dup-head-text">
        <span class="dup-title">{$t("f.dup.title", { scope: shortScope })}</span>
        <span class="dup-sub">
          {$t("f.dup.groups", { count: groups.length })}
          {#if $reclaimable > 0}, {$t("f.dup.reclaimable", { size: formatSize($reclaimable) })}{/if}
        </span>
      </div>
      <div class="dup-actions">
        <button class="dup-btn" onclick={() => keepNewest()}>{$t("f.dup.keepNewest")}</button>
        <button
          class="dup-btn primary"
          disabled={$markedPaths.length === 0}
          onclick={() => (confirming = true)}
        >
          <Trash2 size={13} strokeWidth={2} />
          {$t("f.dup.trashCount", { count: $markedPaths.length })}
        </button>
      </div>
    </div>

    <div class="dup-scroll">
      {#each groups as group (group.hash)}
        {@const kept = keptCount(group, $trashMarks)}
        <div class="group">
          <div class="group-head">
            {$t("f.dup.copies", { count: group.files.length })}
            {#if groupReclaimable(group, $trashMarks) > 0}
              , {$t("f.dup.reclaimable", { size: formatSize(groupReclaimable(group, $trashMarks)) })}
            {/if}
          </div>
          {#each group.files as file (file.path)}
            {@const Icon = entryIcon(entryOf(file))}
            {@const marked = $trashMarks.has(file.path)}
            {@const locked = !marked && kept <= 1}
            <div class="row" class:marked>
              {#if locked}
                <span class="mark keep locked" title={$t("f.dup.oneKept")}>
                  <Lock size={11} strokeWidth={2} />
                  {$t("f.dup.keep")}
                </span>
              {:else}
                <button
                  class="mark {marked ? 'trash' : 'keep'}"
                  onclick={() => toggleTrash(group, file.path)}
                >
                  <span class="box" aria-hidden="true"></span>
                  {marked ? $t("f.dup.trash") : $t("f.dup.keep")}
                </button>
              {/if}
              <span class="name-cell">
                <span class="icon"><Icon size={16} strokeWidth={1.75} /></span>
                <span class="name">{file.name}</span>
              </span>
              <span class="dir">{dirOf(file.path).replace(/^\/home\/[^/]+/, "~")}</span>
              <span class="size">{formatSize(file.size)}</span>
              <span class="mod">{formatModified(file.modified_unix)}</span>
            </div>
          {/each}
        </div>
      {/each}
    </div>
  {/if}
</div>

<ConfirmDialog
  open={confirming}
  title={$t("f.dup.dialogTitle")}
  message={$t("f.dup.dialogBody", { count: $markedPaths.length, size: formatSize($reclaimable) })}
  confirmLabel={$t("f.menu.moveToTrash")}
  onConfirm={() => {
    confirming = false;
    ontrash?.($markedPaths);
    closeDuplicates();
  }}
  onCancel={() => (confirming = false)}
/>

<style>
  .dup {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }

  .dup-state {
    margin: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 2rem;
    text-align: center;
  }
  .dup-state :global(.spin) {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    animation: dup-spin 1s linear infinite;
  }
  @keyframes dup-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .dup-state-title {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .dup-state-hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .dup-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 16px;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .dup-head-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .dup-title {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .dup-sub {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .dup-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .dup-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: var(--height-control, 28px);
    padding: 0 11px;
    border: 1px solid var(--control-border);
    background: var(--control-bg);
    border-radius: var(--radius-input);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
  }
  .dup-btn:hover:not(:disabled) {
    background: var(--control-bg-hover);
  }
  .dup-btn.primary {
    border-color: transparent;
    background: color-mix(in srgb, var(--color-error, #c96a6a) 18%, transparent);
    color: color-mix(in srgb, var(--color-error, #c96a6a) 92%, var(--foreground));
  }
  .dup-btn.primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-error, #c96a6a) 26%, transparent);
  }
  .dup-btn:disabled {
    opacity: 0.45;
  }

  .dup-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 8px 8px 12px;
  }
  .group + .group {
    margin-top: 10px;
  }
  .group-head {
    padding: 6px 8px 2px;
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .row {
    display: grid;
    grid-template-columns: 5.5rem minmax(0, 2fr) minmax(0, 2fr) 5rem 8rem;
    align-items: center;
    gap: 8px;
    height: 2rem;
    padding: 0 8px;
    border-radius: var(--radius-input);
  }
  .row:hover {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .row.marked {
    opacity: 0.62;
  }

  .mark {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 7px 0 5px;
    border: none;
    background: transparent;
    border-radius: var(--radius-chip);
    font-size: 0.6875rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .mark .box {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    border: 1.5px solid color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .mark.keep:hover .box {
    border-color: var(--color-error, #c96a6a);
  }
  .mark.trash {
    color: color-mix(in srgb, var(--color-error, #c96a6a) 90%, var(--foreground));
  }
  .mark.trash .box {
    border-color: var(--color-error, #c96a6a);
    background: var(--color-error, #c96a6a);
  }
  .mark.locked {
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  .name-cell {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .name {
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dir {
    min-width: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 38%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size,
  .mod {
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
