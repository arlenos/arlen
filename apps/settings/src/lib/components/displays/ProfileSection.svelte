<script lang="ts">
  /// Saved-layout list for the Display panel.
  ///
  /// Each entry shows the profile's label and the connectors it
  /// covers. Apply / Rename / Delete sit on the right of the row.
  /// "Save current as new layout…" is the footer action that
  /// snapshots the live monitor topology.
  ///
  /// Apply goes through the same `display_apply_config` pipeline
  /// the freeform editor uses, which means the 15-second revert
  /// modal kicks in for safety. We expose the modal state up to
  /// the Display page via the `onApplied` callback so it can show
  /// the existing `RevertConfirmModal`.

  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import SettingsGroup from "$lib/components/settings/SettingsGroup.svelte";
  import { ConfirmDialog } from "$lib/components/ui/confirm-dialog";
  import type { ApplyHandle, MonitorConfig } from "$lib/stores/displays";
  import { Check, Pencil, Play, Trash2 } from "lucide-svelte";

  interface OutputInfoSummary {
    connector: string;
    make: string;
    model: string;
  }

  interface ProfileSummary {
    id: string;
    label: string;
    outputSet: OutputInfoSummary[];
    lastUsed: string | null;
    isCurrent: boolean;
  }

  interface Props {
    /// Called when an `Apply` triggered an `display_apply_config`
    /// flow with the canonical `{requestId, snapshot}` handle —
    /// the parent page wires that into its 15-second revert
    /// modal so the saved-layout flow gets the same safety net
    /// as the inline editor.
    onApplied: (handle: ApplyHandle) => void;
  }

  let { onApplied }: Props = $props();

  let profiles = $state<ProfileSummary[]>([]);
  let unlistenChanged: UnlistenFn | null = null;

  // "Save current as…" inline form state. The form lives at the
  // bottom of the card; submitting calls `display_profile_save`
  // and refreshes the list.
  let saveLabel = $state("");
  let saving = $state(false);

  // Rename inline-edit state. Only one row may be in edit mode at
  // a time; clicking another row's pencil cancels the previous.
  let renamingId = $state<string | null>(null);
  let renamingLabel = $state("");

  // Delete confirm-dialog state.
  let deleteCandidate = $state<ProfileSummary | null>(null);

  async function reload() {
    try {
      profiles = await invoke<ProfileSummary[]>("display_profiles_list");
    } catch (err) {
      console.warn("display_profiles_list failed:", err);
    }
  }

  onMount(async () => {
    await reload();
    // Refresh whenever the live monitor list changes — that's the
    // only signal the `is_current` flag depends on.
    unlistenChanged = await listen("displays:changed", () => reload());
  });

  onDestroy(() => {
    unlistenChanged?.();
  });

  async function doSave() {
    const label = saveLabel.trim();
    if (!label || saving) return;
    saving = true;
    try {
      await invoke<ProfileSummary>("display_profile_save", { label });
      saveLabel = "";
      await reload();
    } catch (err) {
      console.warn("display_profile_save failed:", err);
    } finally {
      saving = false;
    }
  }

  function onSubmit(e: SubmitEvent) {
    e.preventDefault();
    doSave();
  }

  async function onApply(profile: ProfileSummary) {
    try {
      const handle = await invoke<{
        requestId: string;
        snapshot: MonitorConfig[];
      }>("display_profile_apply", { id: profile.id });
      onApplied(handle);
      await reload();
    } catch (err) {
      console.warn("display_profile_apply failed:", err);
    }
  }

  function startRename(profile: ProfileSummary) {
    renamingId = profile.id;
    renamingLabel = profile.label;
  }

  async function commitRename() {
    if (!renamingId) return;
    const label = renamingLabel.trim();
    if (!label) {
      renamingId = null;
      return;
    }
    try {
      await invoke("display_profile_rename", {
        id: renamingId,
        label,
      });
    } catch (err) {
      console.warn("display_profile_rename failed:", err);
    }
    renamingId = null;
    await reload();
  }

  function cancelRename() {
    renamingId = null;
  }

  async function confirmDelete() {
    if (!deleteCandidate) return;
    const id = deleteCandidate.id;
    deleteCandidate = null;
    try {
      await invoke("display_profile_delete", { id });
    } catch (err) {
      console.warn("display_profile_delete failed:", err);
    }
    await reload();
  }

  function describeOutputs(set: OutputInfoSummary[]): string {
    return set.map((o) => o.connector).join(" + ");
  }

  function describeWhen(iso: string | null): string {
    if (!iso) return "never applied";
    const t = new Date(iso).getTime();
    const seconds = Math.max(0, Math.floor((Date.now() - t) / 1000));
    if (seconds < 60) return "just now";
    if (seconds < 3600) return `${Math.floor(seconds / 60)} min ago`;
    if (seconds < 86_400) return `${Math.floor(seconds / 3600)} h ago`;
    return `${Math.floor(seconds / 86_400)} d ago`;
  }
</script>

<SettingsGroup label="Saved Layouts">
  {#if profiles.length === 0}
    <div class="empty">
      No saved layouts yet. Save your current arrangement below to
      restore it automatically when you re-attach the same monitors.
    </div>
  {:else}
    {#each profiles as p (p.id)}
      <div class="row" class:active={p.isCurrent}>
        <div class="row-meta">
          {#if renamingId === p.id}
            <Input
              value={renamingLabel}
              oninput={(e) => (renamingLabel = (e.currentTarget as HTMLInputElement).value)}
              onkeydown={(e) => {
                if (e.key === "Enter") commitRename();
                if (e.key === "Escape") cancelRename();
              }}
              onblur={commitRename}
              autofocus
            />
          {:else}
            <div class="label-line">
              <span class="label">{p.label}</span>
              {#if p.isCurrent}
                <span class="badge"><Check size={12} strokeWidth={2.5} /> Active</span>
              {/if}
            </div>
            <div class="meta">
              {describeOutputs(p.outputSet)} · {describeWhen(p.lastUsed)}
            </div>
          {/if}
        </div>
        <div class="row-actions">
          <Button
            variant="outline"
            size="sm"
            onclick={() => onApply(p)}
            disabled={p.isCurrent}
            title={p.isCurrent ? "This layout is already active" : "Apply"}
          >
            <Play size={12} strokeWidth={2.5} /> Apply
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onclick={() => startRename(p)}
            aria-label="Rename layout"
            title="Rename"
          >
            <Pencil size={12} strokeWidth={2} />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onclick={() => (deleteCandidate = p)}
            aria-label="Delete layout"
            title="Delete"
          >
            <Trash2 size={12} strokeWidth={2} />
          </Button>
        </div>
      </div>
    {/each}
  {/if}

  <form class="save-row" onsubmit={onSubmit}>
    <Input
      value={saveLabel}
      oninput={(e) => (saveLabel = (e.currentTarget as HTMLInputElement).value)}
      placeholder="New layout name…"
      aria-label="New layout name"
    />
    <Button
      variant="outline"
      onclick={doSave}
      disabled={saving || !saveLabel.trim()}
    >
      Save current
    </Button>
  </form>
</SettingsGroup>

<ConfirmDialog
  open={!!deleteCandidate}
  title="Delete saved layout?"
  message={deleteCandidate
    ? `'${deleteCandidate.label}' will be removed permanently. This does not change the current display setup.`
    : ""}
  confirmLabel="Delete"
  variant="destructive"
  onConfirm={confirmDelete}
  onCancel={() => (deleteCandidate = null)}
/>

<style>
  .empty {
    padding: 16px;
    font-size: 0.85rem;
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 16px;
  }

  .row.active {
    background: color-mix(in srgb, var(--color-accent) 6%, transparent);
  }

  .row-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .label-line {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .label {
    font-size: 0.9rem;
    font-weight: 500;
    color: var(--color-fg-app);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 1px 6px;
    background: color-mix(in srgb, var(--color-accent) 22%, transparent);
    color: var(--color-accent);
    border-radius: var(--radius-chip);
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .meta {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
  }

  .row-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }

  .save-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 16px;
  }

  .save-row :global(.flex) {
    flex: 1;
  }
</style>
