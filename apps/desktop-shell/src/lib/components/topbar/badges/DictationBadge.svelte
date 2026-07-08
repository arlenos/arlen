<script lang="ts">
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
    invoke("stop_dictation")
      .then(() => refresh())
      .catch(() => {});
  }
</script>

<StatusBadge
  visible={active}
  active={active}
  pulsate
  label="Dictating"
  title={`Dictating into ${target}. Click to stop.`}
  onclick={handleClick}
>
  {#snippet icon()}
    <Mic size={12} strokeWidth={2} />
  {/snippet}
</StatusBadge>
