<script lang="ts">
  /// The voice HUD: a calm bottom-centre overlay for the push-to-talk moment. It
  /// shows Listening (a live level meter) -> your transcript -> the agent Thinking ->
  /// its reply, and carries the sovereign line - on-device, held-key only, audited -
  /// the anti-Alexa statement made visible. Mounted once in +layout, inert when idle.
  import { voice } from "$lib/stores/voice";
  import { Mic } from "lucide-svelte";
</script>

{#if $voice.phase !== "idle"}
  {@const v = $voice}
  <div class="vh" role="status" aria-live="polite">
    <div class="vh-card">
      {#if v.phase === "listening" || v.phase === "thinking"}
        <div class="vh-state">
          <span class="vh-mic"><Mic size={15} strokeWidth={2} /></span>
          {#if v.phase === "listening"}
            <span class="vh-meter" aria-hidden="true">
              {#each Array(5) as _, i (i)}<span class="bar" style="--i: {i}"></span>{/each}
            </span>
            <span class="vh-label">Listening</span>
          {:else}
            <span class="vh-pulse" aria-hidden="true"></span>
            <span class="vh-label">Thinking</span>
          {/if}
        </div>
      {/if}

      {#if v.transcript}
        <p class="vh-transcript">{v.transcript}</p>
      {/if}
      {#if v.phase === "replying" && v.reply}
        <p class="vh-reply">{v.reply}</p>
      {/if}

      <div class="vh-foot">
        <span class="vh-dot" class:on={v.phase === "listening"}></span>
        On this device and in your audit log.
      </div>
    </div>
  </div>
{/if}

<style>
  .vh {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 2.5rem;
    z-index: 60;
    display: flex;
    justify-content: center;
    pointer-events: none;
  }
  .vh-card {
    pointer-events: auto;
    min-width: 18rem;
    max-width: min(30rem, calc(100vw - 3rem));
    padding: 0.85rem 1rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-modal, 16px);
    background: var(--color-bg-card, #171717);
    box-shadow: var(--shadow-lg, 0 12px 40px #00000070);
  }
  .vh-state {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .vh-mic {
    display: inline-flex;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .vh-label {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .vh-meter {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    height: 14px;
  }
  .vh-meter .bar {
    width: 3px;
    height: 100%;
    border-radius: 2px;
    background: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    transform-origin: center;
    animation: vh-bar 0.9s ease-in-out infinite;
    animation-delay: calc(var(--i) * 0.12s);
  }
  @keyframes vh-bar {
    0%,
    100% {
      transform: scaleY(0.35);
    }
    50% {
      transform: scaleY(1);
    }
  }
  .vh-pulse {
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    animation: vh-pulse 1.1s ease-in-out infinite;
  }
  @keyframes vh-pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }

  .vh-transcript {
    margin: 0.6rem 0 0;
    font-size: 1rem;
    line-height: 1.4;
    color: var(--color-fg-primary);
  }
  .vh-reply {
    margin: 0.55rem 0 0;
    font-size: 0.9375rem;
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 72%, transparent);
  }

  .vh-foot {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin-top: 0.7rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
  .vh-dot {
    width: 0.45rem;
    height: 0.45rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-fg-primary) 30%, transparent);
  }
  /* The mic-on signal - lit amber exactly while listening (held-key only). */
  .vh-dot.on {
    background: var(--color-warning, #d0a54a);
  }
</style>
