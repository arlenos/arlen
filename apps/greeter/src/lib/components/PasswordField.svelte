<script lang="ts">
  /// The password field: a flat input in the house style (the kit Input /
  /// the Waypointer search), with a quiet reveal toggle and a quiet submit
  /// glyph, both topbar-register icon buttons. No pill, no blur, no solid
  /// circle. This is the gesture that releases the encrypted home, so it
  /// stays plain and unambiguous. A wrong password shakes once and clears;
  /// the parent owns the error flag and the aria-live message.
  import { ArrowRight, Eye, EyeOff, Loader2 } from "@lucide/svelte";

  let {
    value = $bindable(""),
    disabled = false,
    busy = false,
    error = false,
    placeholder = "Password",
    id = "greeter-password",
    onsubmit,
  }: {
    value?: string;
    disabled?: boolean;
    busy?: boolean;
    error?: boolean;
    placeholder?: string;
    id?: string;
    onsubmit: () => void;
  } = $props();

  let reveal = $state(false);
  let inputEl = $state<HTMLInputElement | null>(null);

  export function focus() {
    inputEl?.focus();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && value.length > 0 && !busy) {
      e.preventDefault();
      onsubmit();
    }
  }
</script>

<div class="field" class:error>
  <input
    bind:this={inputEl}
    {id}
    class="entry"
    type={reveal ? "text" : "password"}
    bind:value
    {placeholder}
    disabled={disabled || busy}
    autocomplete="current-password"
    spellcheck="false"
    aria-label="Password"
    aria-invalid={error}
    {onkeydown}
  />
  <button
    type="button"
    class="icon"
    id="greeter-password-reveal"
    aria-label={reveal ? "Hide password" : "Show password"}
    aria-pressed={reveal}
    tabindex={-1}
    disabled={disabled || busy}
    onclick={() => (reveal = !reveal)}
  >
    {#if reveal}
      <EyeOff size={16} strokeWidth={1.5} />
    {:else}
      <Eye size={16} strokeWidth={1.5} />
    {/if}
  </button>
  <button
    type="button"
    class="icon submit"
    id="greeter-password-submit"
    aria-label="Sign in"
    disabled={disabled || busy || value.length === 0}
    onclick={onsubmit}
  >
    {#if busy}
      <Loader2 size={16} strokeWidth={2} class="spin" />
    {:else}
      <ArrowRight size={16} strokeWidth={2} />
    {/if}
  </button>
</div>

<style>
  /* The flat house input: bg-input, a 1px hairline, the input radius, the
     prominent control height. Same recipe as the kit Input. */
  .field {
    display: flex;
    align-items: center;
    gap: 0.125rem;
    width: 100%;
    height: calc(var(--height-control-prominent, 36px) * var(--greeter-scale, 1));
    padding: 0 0.25rem 0 0.75rem;
    border-radius: var(--radius-input);
    background: var(--color-bg-input);
    border: 1px solid var(--color-border);
    transition: border-color var(--duration-fast) var(--ease-out);
  }
  .field:focus-within {
    border-color: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .field.error {
    border-color: var(--color-error);
    animation: greeter-shake var(--duration-medium) var(--ease-out);
  }

  .entry {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: none;
    background: transparent;
    outline: none;
    color: var(--foreground);
    font-size: calc(0.875rem * var(--greeter-scale, 1));
  }
  .entry::placeholder {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  /* Topbar-register icon buttons: quiet, flat, the input radius. The submit
     is the same register, only a touch brighter, never a solid pill. */
  .icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    width: calc(1.75rem * var(--greeter-scale, 1));
    height: calc(1.75rem * var(--greeter-scale, 1));
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out),
      opacity var(--duration-fast) var(--ease-out);
  }
  .icon:hover:not(:disabled) {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }
  .submit {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .icon:disabled {
    opacity: 0.4;
  }

  :global(.spin) {
    animation: greeter-spin 0.8s linear infinite;
  }
  @keyframes greeter-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes greeter-shake {
    0%, 100% { transform: translateX(0); }
    20% { transform: translateX(-6px); }
    40% { transform: translateX(6px); }
    60% { transform: translateX(-3px); }
    80% { transform: translateX(3px); }
  }
</style>
