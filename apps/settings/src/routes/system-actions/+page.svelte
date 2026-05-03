<script lang="ts">
  /// System Actions settings page (Sprint B).
  ///
  /// Configures `compositor.toml [system_actions]` per-action command
  /// bindings. Each row shows the current command (defaults are NOT
  /// written into the file — the compositor falls back to its
  /// `default_system_actions()` table when a key is missing) and a
  /// `CommandStringEditor` for inline editing.
  ///
  /// Reset-per-action removes the user's TOML override so the
  /// compositor picks the default again. Reset-all confirms first.

  import { onMount } from "svelte";
  import { ConfirmDialog } from "$lib/components/ui/confirm-dialog";
  import { Button } from "$lib/components/ui/button";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { CommandStringEditor } from "$lib/components/ui/command-string-editor";
  import {
    compositor,
    SYSTEM_ACTIONS,
    SYSTEM_ACTION_CATEGORIES,
    actionsByCategory,
    type SystemActionDef,
  } from "$lib/stores/systemActions";

  onMount(() => {
    compositor.load();
  });

  /// Resolved current value for an action: the user's
  /// `[system_actions].<key>` if set, otherwise the built-in default.
  function currentValue(action: SystemActionDef): string {
    const overrides =
      ($compositor.data?.system_actions as Record<string, string> | undefined) ??
      {};
    const explicit = overrides[action.key];
    return explicit ?? action.default;
  }

  function isModified(action: SystemActionDef): boolean {
    const overrides =
      ($compositor.data?.system_actions as Record<string, string> | undefined) ??
      {};
    return overrides[action.key] !== undefined &&
      overrides[action.key] !== action.default;
  }

  async function setAction(action: SystemActionDef, value: string) {
    if (value === action.default) {
      // User typed the default back in: drop the override so the
      // file stays clean.
      await compositor.reset(`system_actions.${action.key}`);
    } else {
      await compositor.setValue(`system_actions.${action.key}`, value);
    }
  }

  async function resetAction(action: SystemActionDef) {
    await compositor.reset(`system_actions.${action.key}`);
  }

  let resetAllOpen = $state(false);

  async function resetAll() {
    resetAllOpen = false;
    // Drop the entire [system_actions] section so the compositor
    // falls back to defaults across the board.
    await compositor.reset("system_actions");
  }

  const modifiedCount = $derived.by(() => {
    return SYSTEM_ACTIONS.filter((a) => isModified(a)).length;
  });
</script>

<SettingsPage
  title="System Actions"
  description="Commands triggered by hardware Fn-row keys and system shortcuts. Edit a command to override the default; reset clears the override."
>
  {#if modifiedCount > 0}
    <div class="header-actions">
      <span class="modified-badge">
        {modifiedCount}
        {modifiedCount === 1 ? "override" : "overrides"} active
      </span>
      <Button
        variant="ghost"
        size="sm"
        onclick={() => (resetAllOpen = true)}
      >
        Reset all to defaults
      </Button>
    </div>
  {/if}

  {#each SYSTEM_ACTION_CATEGORIES as category (category)}
    <Group label={category}>
      {#each actionsByCategory(category) as action (action.key)}
        <Row
          label={action.label}
          description={action.description}
          id={`action-${action.key}`}
        >
          {#snippet control()}
            <div class="editor-wrap">
              <CommandStringEditor
                value={currentValue(action)}
                defaultValue={action.default}
                onchange={(v) => setAction(action, v)}
              />
            </div>
          {/snippet}
        </Row>
      {/each}
    </Group>
  {/each}

  <div class="footer-note">
    <strong>Command grammar:</strong>
    <code>shell:event_name</code> dispatches a shell-overlay event
    (e.g. <code>shell:brightness_up</code>),
    <code>spawn:command</code> runs a shell command. Bare strings
    are also accepted and run via <code>/bin/sh -c</code> for
    compatibility.
  </div>
</SettingsPage>

<ConfirmDialog
  open={resetAllOpen}
  title="Reset all system actions?"
  message="The whole [system_actions] section will be removed from compositor.toml so every action falls back to the built-in default."
  confirmLabel="Reset all"
  variant="destructive"
  onConfirm={resetAll}
  onCancel={() => (resetAllOpen = false)}
/>

<style>
  .editor-wrap {
    width: 360px;
    max-width: 100%;
  }

  .header-actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.75rem;
    margin-bottom: -0.5rem;
  }

  .modified-badge {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }

  .footer-note {
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    padding: 0.75rem 1rem;
    border: 1px dashed
      color-mix(in srgb, var(--foreground) 14%, transparent);
    border-radius: var(--radius-chip);
  }

  .footer-note code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.7rem;
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    padding: 0.1rem 0.35rem;
    border-radius: 4px;
  }
</style>
