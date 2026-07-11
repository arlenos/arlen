<script lang="ts">
  /// The composer-foot autonomy dial: how freely the assistant acts on its
  /// own. It reads the live state from the agent (`action_state`: the baseline
  /// `action_mode` plus the Tim-gated `executor_live` master) and flips the
  /// baseline via `ai_set_action_mode`. The chip shows the EFFECTIVE state
  /// honestly: a "supervised" baseline still only suggests while the executor
  /// master is off, so the chip says "Suggests only" until both line up.
  /// The master itself is not flipped here - enabling the agent to act on your
  /// real data is a deliberate Settings action, not a casual chat toggle.
  /// Svelte-5 IPC caveat: the loaded state lives in a writable store.
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { ChevronDown } from "@lucide/svelte";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import { openTransparency } from "$lib/stores/transparency";

  interface ActionState {
    action_mode: "suggest" | "supervised";
    executor_live: boolean;
  }

  // Null until the first read settles; the chip stays hidden so nothing flashes.
  const state = writable<ActionState | null>(null);

  async function load() {
    try {
      const s = JSON.parse(await invoke<string>("action_state")) as Partial<ActionState>;
      state.set({
        action_mode: s.action_mode === "supervised" ? "supervised" : "suggest",
        executor_live: s.executor_live === true,
      });
    } catch {
      state.set(null);
    }
  }
  onMount(load);

  // The effective behaviour: acting only when the baseline is supervised AND
  // the executor master is live. Otherwise the assistant only suggests.
  const acting = $derived($state?.action_mode === "supervised" && $state?.executor_live === true);
  const dialValue = $derived($state?.action_mode ?? "suggest");

  async function setMode(mode: string) {
    const m = mode === "supervised" ? "supervised" : "suggest";
    const prev = $state;
    // Optimistic; the agent re-reads ai.toml live, so the next load confirms.
    state.update((s) => (s ? { ...s, action_mode: m } : s));
    try {
      const ok = (await invoke<string>("ai_set_action_mode", { mode: m })) === "ok";
      if (!ok && prev) state.set(prev);
    } catch {
      if (prev) state.set(prev);
    }
  }
</script>

{#if $state}
  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        <button type="button" class="dial" aria-label={$t("h.autonomy.aria")} {...props}>
          <span class="glyph" aria-hidden="true">{acting ? "◐" : "○"}</span>
          <span class="label">{acting ? $t("h.autonomy.acts") : $t("h.autonomy.suggests")}</span>
          <ChevronDown size={12} strokeWidth={2} class="dial-chev" />
        </button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content side="top" align="end" class="dial-menu">
      <DropdownMenu.Label>{$t("h.autonomy.label")}</DropdownMenu.Label>
      <DropdownMenu.RadioGroup value={dialValue} onValueChange={setMode}>
        <DropdownMenu.RadioItem value="suggest">
          {$t("h.autonomy.suggestFull")}
        </DropdownMenu.RadioItem>
        <DropdownMenu.RadioItem value="supervised">
          {$t("h.autonomy.actsFull")}
        </DropdownMenu.RadioItem>
      </DropdownMenu.RadioGroup>
      {#if $state.action_mode === "supervised" && !$state.executor_live}
        <p class="note">{$t("h.autonomy.hint")}</p>
      {/if}
      <DropdownMenu.Separator />
      <DropdownMenu.Item onclick={() => openTransparency()}>
        Manage in Transparency
      </DropdownMenu.Item>
    </DropdownMenu.Content>
  </DropdownMenu.Root>
{/if}

<style>
  .dial {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
    height: var(--height-control, 28px);
    padding: 0 0.5rem;
    border: none;
    border-radius: var(--radius-button);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-size: var(--text-xs);
    transition: color var(--duration-fast) var(--ease-out);
  }
  .dial:hover {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .glyph {
    color: var(--color-success);
  }
  .label {
    flex-shrink: 0;
  }
  :global(.dial-chev) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .note {
    margin: 0;
    padding: 0.25rem 0.5rem 0.375rem;
    max-width: 15rem;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
