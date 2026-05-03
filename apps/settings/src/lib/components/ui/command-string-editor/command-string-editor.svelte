<script lang="ts">
  /// Inline editor for system-action command strings.
  ///
  /// Read-only display by default with a pencil-edit affordance.
  /// Click → Input replaces the display with Save/Cancel buttons.
  /// Validates the `shell:` / `spawn:` / bare prefix grammar that
  /// the compositor's `Action::System` dispatch expects (compositor
  /// #29 / CC2). Shows a reset-to-default button when the value
  /// differs from `defaultValue`.
  ///
  /// Used by the System Actions page for editing per-action
  /// commands (Volume Up = `spawn:wpctl …`, Brightness Up =
  /// `shell:brightness_up`, etc.).

  import { Pencil, Check, X, RotateCcw } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";

  let {
    value,
    defaultValue,
    onchange,
    placeholder = "shell:event_name or spawn:command",
  }: {
    value: string;
    /// When supplied, a reset button appears once `value !== defaultValue`.
    /// The reset emits `onchange(defaultValue)`; this component does
    /// not delete the underlying TOML key (the consumer's store does).
    defaultValue?: string;
    onchange: (value: string) => void;
    placeholder?: string;
  } = $props();

  let editing = $state(false);
  let draft = $state("");
  let error = $state<string | null>(null);

  function startEdit() {
    draft = value;
    editing = true;
    error = null;
  }

  function cancel() {
    editing = false;
    error = null;
  }

  /// Validate the prefix grammar. Returns null on valid, otherwise
  /// the error message to show inline. We accept bare strings too
  /// because legacy user configs may rely on them — the compositor
  /// dispatches them as raw `/bin/sh -c` for backward compat.
  function validate(s: string): string | null {
    const trimmed = s.trim();
    if (trimmed.length === 0) return "Empty command";
    if (trimmed.startsWith("shell:")) {
      const event = trimmed.slice("shell:".length).trim();
      if (event.length === 0) return "shell: needs an event name";
    } else if (trimmed.startsWith("spawn:")) {
      const cmd = trimmed.slice("spawn:".length).trim();
      if (cmd.length === 0) return "spawn: needs a command";
    }
    return null;
  }

  function save() {
    const trimmed = draft.trim();
    const err = validate(trimmed);
    if (err) {
      error = err;
      return;
    }
    onchange(trimmed);
    editing = false;
  }

  function reset() {
    if (defaultValue !== undefined) {
      onchange(defaultValue);
    }
  }

  const isModified = $derived(
    defaultValue !== undefined && value !== defaultValue,
  );

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      save();
    } else if (e.key === "Escape") {
      e.preventDefault();
      cancel();
    }
  }
</script>

{#if editing}
  <div class="editor">
    <Input
      bind:value={draft}
      {placeholder}
      onkeydown={onKeydown}
      autofocus
    />
    {#if error}
      <div class="error" role="alert">{error}</div>
    {/if}
    <div class="buttons">
      <Button variant="ghost" size="sm" onclick={cancel}>
        <X size={14} />
        Cancel
      </Button>
      <Button size="sm" onclick={save}>
        <Check size={14} />
        Save
      </Button>
    </div>
  </div>
{:else}
  <div class="display">
    <code class="value" title={value}>{value}</code>
    <div class="actions">
      {#if isModified}
        <Button
          variant="ghost"
          size="icon-sm"
          onclick={reset}
          aria-label="Reset to default"
          title="Reset to default"
        >
          <RotateCcw size={14} />
        </Button>
      {/if}
      <Button
        variant="ghost"
        size="icon-sm"
        onclick={startEdit}
        aria-label="Edit"
      >
        <Pencil size={14} />
      </Button>
    </div>
  </div>
{/if}

<style>
  .display {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    min-width: 0;
  }

  .value {
    flex: 1;
    min-width: 0;
    padding: 0.375rem 0.5rem;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border-radius: var(--radius-chip);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions {
    display: flex;
    gap: 0.125rem;
    flex-shrink: 0;
  }

  .editor {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    width: 100%;
  }

  .error {
    font-size: 0.75rem;
    color: var(--destructive);
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 0.25rem;
  }
</style>
