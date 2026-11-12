<script lang="ts">
  /// The composer-foot autonomy chip: how freely the assistant acts on its own.
  /// It reads the engine's `action_state` (the baseline `action_mode` plus the
  /// `executor_live` master) and shows the EFFECTIVE state: a "supervised"
  /// baseline still only suggests while the master is off, so the chip says
  /// "Suggests only" until both line up.
  /// A readout and not a switch. Both values belong to the config-broker, which
  /// admits Settings as its one writer, so a press opens the transparency
  /// drawer, where the way to Settings is.
  /// Svelte-5 IPC caveat: the loaded state lives in a writable store.
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { openTransparency } from "$lib/stores/transparency";

  interface ActionState {
    action_mode: "suggest" | "supervised";
    executor_live: boolean;
  }

  // Null until the first read settles, and null again when it fails: the chip
  // stays hidden rather than state a position nobody measured.
  const state = writable<ActionState | null>(null);

  onMount(async () => {
    try {
      const s = JSON.parse(await invoke<string>("action_state")) as Partial<ActionState> | null;
      state.set(
        s
          ? {
              action_mode: s.action_mode === "supervised" ? "supervised" : "suggest",
              executor_live: s.executor_live === true,
            }
          : null,
      );
    } catch {
      state.set(null);
    }
  });

  // Acting only when the baseline is supervised AND the executor master is live.
  const acting = $derived($state?.action_mode === "supervised" && $state?.executor_live === true);
</script>

{#if $state}
  <button type="button" class="dial" aria-label={$t("h.autonomy.aria")} onclick={() => openTransparency()}>
    <span class="glyph" aria-hidden="true">{acting ? "◐" : "○"}</span>
    <span class="label">{acting ? $t("h.autonomy.acts") : $t("h.autonomy.suggests")}</span>
  </button>
{/if}

<style>
  .dial {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    min-width: 0;
    height: var(--height-control, 30px);
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
</style>
