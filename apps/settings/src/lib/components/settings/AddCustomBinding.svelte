<script lang="ts">
  /// Modal to add a custom binding. Three modes:
  /// * App:    user types a command (becomes `spawn:<cmd>`)
  /// * Command: same as App, but presented as raw shell input
  /// * Shell:  user picks a known shell-overlay event (`shell:<name>`)

  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import KeyCapture from "./KeyCapture.svelte";

  type Mode = "app" | "command" | "shell";

  type Props = {
    open: boolean;
    onClose: () => void;
    onAdd: (binding: string, action: string) => void;
  };

  let { open, onClose, onAdd }: Props = $props();

  let mode = $state<Mode>("app");
  let commandInput = $state("");
  let shellEvent = $state("waypointer_open");
  let binding = $state<string | null>(null);
  let capturing = $state(false);
  let error = $state<string | null>(null);

  /// `value`/`label` shape matches the ui-kit Select component so we
  /// can bind the list straight to it without a mapping step.
  const SHELL_EVENTS = [
    { value: "waypointer_open", label: "Open Waypointer" },
    { value: "waypointer_toggle", label: "Toggle Waypointer" },
  ];

  function reset(): void {
    mode = "app";
    commandInput = "";
    shellEvent = "waypointer_open";
    binding = null;
    capturing = false;
    error = null;
  }

  function composeAction(): string | null {
    if (mode === "app" || mode === "command") {
      const cmd = commandInput.trim();
      if (!cmd) return null;
      return `spawn:${cmd}`;
    }
    return `shell:${shellEvent}`;
  }

  function save(): void {
    error = null;
    const action = composeAction();
    if (!action) {
      error = "Please enter a command";
      return;
    }
    if (!binding) {
      error = "Please capture a key combination";
      return;
    }
    onAdd(binding, action);
    reset();
  }

  function cancel(): void {
    reset();
    onClose();
  }

  function onCapture(combo: string): void {
    binding = combo;
    capturing = false;
  }
</script>

{#if open}
  <div
    class="fixed inset-0 z-40 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
  >
    <div
      class="w-full max-w-md rounded-[var(--radius)] border border-border bg-card p-6 shadow-lg"
    >
      <h2 class="mb-4 text-base font-semibold">Add custom binding</h2>

      <div class="mb-4 flex gap-1 rounded-[var(--radius-sm)] bg-muted p-1">
        {#each ["app", "command", "shell"] as const as tab (tab)}
          <button
            type="button"
            class="flex-1 rounded-[var(--radius-sm)] px-3 py-1.5 text-xs capitalize transition-colors"
            class:bg-background={mode === tab}
            class:shadow-sm={mode === tab}
            onclick={() => (mode = tab)}
          >
            {tab === "app" ? "App" : tab === "command" ? "Command" : "Shell Action"}
          </button>
        {/each}
      </div>

      {#if mode === "app" || mode === "command"}
        <label class="mb-4 block">
          <span class="mb-1 block text-xs text-muted-foreground">
            {mode === "app" ? "Application or binary name" : "Shell command"}
          </span>
          <Input
            bind:value={commandInput}
            placeholder={mode === "app" ? "firefox" : "pkill -USR1 waybar"}
          />
        </label>
      {:else}
        <label class="mb-4 block">
          <span class="mb-1 block text-xs text-muted-foreground">
            Shell event
          </span>
          <PopoverSelect
            value={shellEvent}
            options={SHELL_EVENTS}
            onchange={(v) => (shellEvent = v)}
            ariaLabel="Shell event"
            width="100%"
          />
        </label>
      {/if}

      <label class="mb-4 block">
        <span class="mb-1 block text-xs text-muted-foreground">Binding</span>
        <button
          type="button"
          class="inline-flex min-h-10 w-full items-center justify-center rounded-[var(--radius-sm)] border border-border bg-background px-3 py-2 font-mono text-sm transition-colors hover:bg-muted"
          onclick={() => (capturing = true)}
        >
          {binding ?? "Click to capture…"}
        </button>
      </label>

      {#if error}
        <p class="mb-3 text-xs text-destructive">{error}</p>
      {/if}

      <div class="flex justify-end gap-2">
        <Button variant="ghost" onclick={cancel}>Cancel</Button>
        <Button onclick={save}>Add</Button>
      </div>
    </div>
  </div>

  <KeyCapture
    open={capturing}
    onCapture={onCapture}
    onCancel={() => (capturing = false)}
  />
{/if}
