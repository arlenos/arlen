<script lang="ts">
  /// The strong-factor affordance under the password field. At the cold
  /// login only the factors that can release the encrypted home are offered:
  /// the password above and a hardware key (FIDO2 / TPM2). Phone proximity
  /// and face are convenience re-unlock of the lock screen only, never here
  /// (lockscreen-plan.md Decided §5). When a hardware login is in flight the
  /// row is a quiet waiting prompt with a flat opacity pulse, no glow.
  import { KeyRound } from "@lucide/svelte";

  let {
    available = false,
    waiting = false,
    onbegin,
    oncancel,
  }: {
    available?: boolean;
    waiting?: boolean;
    onbegin: () => void;
    oncancel: () => void;
  } = $props();
</script>

{#if waiting}
  <div class="waiting" role="status" aria-live="polite">
    <span class="pulse"><KeyRound size={16} strokeWidth={1.5} /></span>
    <span class="prompt">Touch your security key</span>
    <button type="button" class="link" id="greeter-factor-cancel" onclick={oncancel}>
      Use password instead
    </button>
  </div>
{:else if available}
  <button type="button" class="trigger" id="greeter-factor-begin" onclick={onbegin}>
    <KeyRound size={15} strokeWidth={1.5} />
    Use a security key
  </button>
{/if}

<style>
  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    height: var(--height-control, 28px);
    padding: 0 0.5rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: calc(0.8125rem * var(--greeter-scale, 1));
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .trigger:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  .waiting {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    color: var(--foreground);
  }
  /* A flat, monochrome chip that pulses by opacity only. No glow ring. */
  .pulse {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2rem;
    height: 2rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
    animation: greeter-fade 1.6s var(--ease-in-out) infinite;
  }
  .prompt {
    font-size: calc(0.875rem * var(--greeter-scale, 1));
  }
  .link {
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: calc(0.75rem * var(--greeter-scale, 1));
  }
  .link:hover {
    color: var(--foreground);
  }

  @keyframes greeter-fade {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.45; }
  }
</style>
