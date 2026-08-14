<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Top-bar dictation indicator (shell-voice-plan.md): the mic-as-audited signal
  /// while you dictate speech into a field. Shown only while dictation runs; a click
  /// stops it. On-device, on only while dictating, audited. Mirrors CaptureBadge.
  ///
  /// Mock-vs-live: the dictation-state signal (the STT-into-field pipeline -> shell)
  /// is a coder seam. Under vite dev the badge shows a fixture so the surface renders;
  /// on metal without the command it stays invisible (no cry-wolf), like CaptureBadge.
  import { StatusBadge } from "@arlen/ui-kit/components/topbar";
  import { Mic } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { shellAction } from "$lib/shellAction";
  import { onMount } from "svelte";

  interface DictationStatus {
    active: boolean;
    targetLabel?: string;
  }

  let active = $state(false);
  let target = $state("a text field");
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  onMount(() => {
    refresh();
    pollInterval = setInterval(refresh, 4_000);
    return () => {
      if (pollInterval) clearInterval(pollInterval);
    };
  });

  async function refresh() {
    try {
      const s = await invoke<DictationStatus>("dictation_status");
      active = s.active;
      target = s.targetLabel ?? "a text field";
    } catch {
      if (import.meta.env.DEV) {
        active = true;
        target = "Text editor";
      }
    }
  }

  function handleClick() {
    // The one press this badge offers, and a refused one is the worst kind of
    // silence here: the badge stays lit because the mic IS still listening, so
    // the surface looks identical whether the click worked or did nothing at
    // all. The line says which, and the re-read means the badge reports the
    // dictation daemon rather than the outcome that was hoped for.
    //
    // No host registers `stop_dictation` yet - it is a recorded seam, "no speech
    // engine". On metal that keeps the badge invisible (the status read fails, so
    // `active` stays false and there is nothing to click), so this cannot cry
    // wolf; under the dev fixture it fires, which is the honest answer there.
    void shellAction("stop_dictation", {}, "sh.badge.errDictation").then(
      () => void refresh(),
    );
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  pulsate
  label={$t("sh.badge.dictating")}
  title={$t("sh.badge.dictatingInto", { target })}
  onclick={handleClick}
>
  {#snippet icon()}
    <Mic size={12} strokeWidth={2} />
  {/snippet}
</StatusBadge>
