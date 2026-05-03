<script lang="ts">
  /// Modal overlay that captures the next key combination the user
  /// presses. Two-stage UX:
  ///
  /// 1. User presses a combo. We record it, stop listening to the
  ///    keyboard, and debounce a D-Bus conflict query so modal stays
  ///    snappy on a second keypress.
  /// 2. If no conflicts, a single "Confirm" button commits the combo.
  ///    If conflicts exist, a warning plus "Use Anyway" / "Cancel" lets
  ///    the user override knowingly.
  ///
  /// The keydown listener runs on `window` with `capture: true` so
  /// typical global bindings (Tab navigation, Ctrl+R refresh) don't
  /// fire while the modal is open. Escape always cancels.

  import { AlertTriangle } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button";
  import {
    queryLiveConflicts,
    type LiveConflict,
  } from "$lib/stores/keybindings";

  type Props = {
    open: boolean;
    onCapture: (binding: string) => void;
    onCancel: () => void;
  };

  let { open, onCapture, onCancel }: Props = $props();

  const MODIFIER_KEYS = ["Meta", "Control", "Alt", "Shift", "OS"];

  /// Live preview of modifiers currently being held (stage 1).
  let livePreview = $state("");
  /// Set once the user commits a complete combo; UI flips to review.
  let captured = $state<string | null>(null);
  /// Conflict lookup state — `null` while we're still waiting for the
  /// debounce; `[]` once the query returns empty; non-empty once a
  /// collision is found.
  let conflicts = $state<LiveConflict[] | null>(null);
  let checking = $state(false);

  let conflictTimer: ReturnType<typeof setTimeout> | null = null;

  function prettyKey(key: string): string {
    if (key === " ") return "Space";
    if (key.length === 1) return key.toUpperCase();
    return key;
  }

  function composeModifiers(e: KeyboardEvent): string[] {
    const parts: string[] = [];
    if (e.metaKey) parts.push("Super");
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.altKey) parts.push("Alt");
    if (e.shiftKey) parts.push("Shift");
    return parts;
  }

  function scheduleConflictCheck(binding: string): void {
    if (conflictTimer) clearTimeout(conflictTimer);
    checking = true;
    conflictTimer = setTimeout(async () => {
      const result = await queryLiveConflicts(binding);
      // If the user has cancelled / recaptured while we were waiting,
      // throw the answer away.
      if (captured === binding) {
        conflicts = result;
        checking = false;
      }
    }, 150);
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (!open) return;
    e.preventDefault();
    e.stopPropagation();

    if (e.key === "Escape") {
      onCancel();
      return;
    }

    // While the user is reviewing a captured binding, ignore stray
    // keys — they'd otherwise recapture and discard the in-flight
    // conflict query.
    if (captured !== null) return;

    const mods = composeModifiers(e);
    livePreview = mods.join("+") + (mods.length ? "+…" : "…");

    if (MODIFIER_KEYS.includes(e.key)) return;
    const combo = [...mods, prettyKey(e.key)].join("+");
    captured = combo;
    conflicts = null;
    scheduleConflictCheck(combo);
  }

  function recapture(): void {
    captured = null;
    conflicts = null;
    if (conflictTimer) clearTimeout(conflictTimer);
    livePreview = "…";
  }

  function confirm(): void {
    if (captured) onCapture(captured);
  }

  $effect(() => {
    if (open) {
      livePreview = "…";
      captured = null;
      conflicts = null;
      checking = false;
      window.addEventListener("keydown", handleKeydown, { capture: true });
      return () => {
        if (conflictTimer) clearTimeout(conflictTimer);
        window.removeEventListener("keydown", handleKeydown, { capture: true });
      };
    }
  });

  const hasConflict = $derived(!!conflicts && conflicts.length > 0);
</script>

{#if open}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
    role="dialog"
    aria-modal="true"
    aria-labelledby="keycapture-title"
  >
    <div
      class="w-full max-w-md rounded-[var(--radius-input)] border border-border bg-card p-6 shadow-lg"
    >
      <h2 id="keycapture-title" class="mb-2 text-base font-semibold">
        Press a key combination
      </h2>
      <p class="mb-4 text-sm text-muted-foreground">
        {captured
          ? "Press keys to recapture, or confirm below."
          : "Press the keys you want to bind. Press Escape to cancel."}
      </p>
      <div
        class="flex min-h-12 items-center justify-center rounded-[var(--radius-chip)] border border-border bg-background px-4 py-3 font-mono text-sm"
        class:border-destructive={hasConflict}
      >
        {captured ?? livePreview}
      </div>

      {#if captured}
        <div class="mt-4">
          {#if checking}
            <p class="text-xs text-muted-foreground">
              Checking for conflicts…
            </p>
          {:else if hasConflict && conflicts}
            <div
              class="flex items-start gap-2 rounded-[var(--radius-chip)] border border-destructive/40 bg-destructive/10 p-3 text-xs"
            >
              <AlertTriangle
                class="mt-0.5 h-4 w-4 shrink-0 text-destructive"
              />
              <div class="text-destructive">
                <div class="font-medium">
                  Already bound to {conflicts.length === 1
                    ? "another action"
                    : `${conflicts.length} other actions`}:
                </div>
                <ul class="mt-1 list-disc pl-4">
                  {#each conflicts as c (c.existingAction)}
                    <li>
                      <span class="font-mono">{c.existingAction}</span>
                      <span class="text-muted-foreground"
                        >({c.existingScope})</span
                      >
                    </li>
                  {/each}
                </ul>
              </div>
            </div>
          {:else if conflicts !== null}
            <p class="text-xs text-muted-foreground">
              No conflicts detected.
            </p>
          {/if}

          <div class="mt-4 flex justify-end gap-2">
            <Button variant="ghost" onclick={recapture}>Recapture</Button>
            <Button variant="ghost" onclick={onCancel}>Cancel</Button>
            {#if hasConflict}
              <Button variant="destructive" onclick={confirm}>
                Use anyway
              </Button>
            {:else}
              <Button onclick={confirm} disabled={checking}>
                Confirm
              </Button>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}
