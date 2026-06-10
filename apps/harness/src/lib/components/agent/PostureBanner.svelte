<script lang="ts">
  /// Acting-posture banner for the agent dashboard: disabled / acting /
  /// suggest-only, derived from the capability read (`ai.toml`). Spans the
  /// full section grid so it shares the dashboard's width and left edge.
  import { Eye, PowerOff, Sparkles } from "@lucide/svelte";
  import type { Capability } from "$lib/capability";

  let { capability }: { capability: Capability } = $props();

  const mode = $derived(
    !capability.enabled ? "off" : capability.executorLive ? "act" : "suggest",
  );

  const META = {
    off: {
      icon: PowerOff,
      title: "AI layer disabled",
      sub: "The agent does nothing until it is enabled in Settings → AI.",
    },
    act: {
      icon: Sparkles,
      title: "Acting",
      sub: "The agent writes safe, reversible curation automatically. Review each action below and undo it if you want.",
    },
    suggest: {
      icon: Eye,
      title: "Suggest-only",
      sub: "The agent computes and proposes curation but writes nothing yet. The activity below is what it observed; turn on the executor in Settings → AI to let it act.",
    },
  } as const;
  const meta = $derived(META[mode]);
</script>

<div class="posture span-full" data-mode={mode}>
  <meta.icon size={16} strokeWidth={1.75} />
  <div class="posture-text">
    <span class="posture-title">{meta.title}</span>
    <span class="posture-sub">{meta.sub}</span>
  </div>
</div>

<style>
  .posture {
    display: flex;
    align-items: flex-start;
    gap: var(--space-row, 0.75rem);
    padding: var(--space-row, 0.75rem);
    border-radius: var(--radius-card);
    border: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .posture :global(svg) {
    flex-shrink: 0;
    margin-top: 0.125rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .posture[data-mode="act"] {
    border-color: color-mix(in srgb, var(--color-accent) 35%, transparent);
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
  }
  .posture[data-mode="act"] :global(svg) {
    color: var(--color-accent);
  }
  .posture-text {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    min-width: 0;
  }
  .posture-title {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .posture-sub {
    font-size: 0.78rem;
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
</style>
