<script lang="ts">
  /// A functional on-screen keyboard for password entry without a hardware
  /// keyboard (a11y at login is mandatory). It drives the password input
  /// directly by id and dispatches the input/keydown events Svelte's
  /// binding listens for, so it stays decoupled from the field component.
  /// Alphanumeric plus a common-symbol row and a caps shift; Enter submits.
  let { targetId = "greeter-password" }: { targetId?: string } = $props();

  let shift = $state(false);

  const ROWS = [
    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"],
    ["q", "w", "e", "r", "t", "y", "u", "i", "o", "p"],
    ["a", "s", "d", "f", "g", "h", "j", "k", "l"],
    ["z", "x", "c", "v", "b", "n", "m"],
    [".", "_", "-", "@", "!", "?", "#", "/", "+"],
  ];

  function field(): HTMLInputElement | null {
    const el = document.getElementById(targetId);
    return el instanceof HTMLInputElement ? el : null;
  }

  function emitInput(el: HTMLInputElement) {
    el.dispatchEvent(new Event("input", { bubbles: true }));
  }

  function type(ch: string) {
    const el = field();
    if (!el) return;
    el.value += shift && ch.length === 1 ? ch.toUpperCase() : ch;
    emitInput(el);
    el.focus();
  }
  function backspace() {
    const el = field();
    if (!el) return;
    el.value = el.value.slice(0, -1);
    emitInput(el);
    el.focus();
  }
  function enter() {
    const el = field();
    if (!el) return;
    el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
  }
</script>

<div class="osk" role="group" aria-label="On-screen keyboard">
  {#each ROWS as row, r (r)}
    <div class="krow">
      {#if r === 3}
        <button
          type="button"
          class="key wide"
          class:active={shift}
          aria-pressed={shift}
          aria-label="Shift"
          onclick={() => (shift = !shift)}>⇧</button>
      {/if}
      {#each row as k (k)}
        <button type="button" class="key" onclick={() => type(k)}>
          {shift && /[a-z]/.test(k) ? k.toUpperCase() : k}
        </button>
      {/each}
      {#if r === 3}
        <button type="button" class="key wide" aria-label="Backspace" onclick={backspace}>⌫</button>
      {/if}
    </div>
  {/each}
  <div class="krow">
    <button type="button" class="key space" aria-label="Space" onclick={() => type(" ")}></button>
    <button type="button" class="key enter" aria-label="Enter" onclick={enter}>Enter</button>
  </div>
</div>

<style>
  /* Flat panel, the house recipe: shell surface, hairline, card radius, a
     quiet float shadow. No blur. */
  .osk {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    width: min(40rem, 92vw);
    padding: 0.75rem;
    border-radius: var(--radius-card);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    box-shadow: var(--shadow-md);
  }
  .krow {
    display: flex;
    justify-content: center;
    gap: 0.4rem;
  }
  .key {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 2.25rem;
    height: 2.5rem;
    padding: 0 0.5rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    border-radius: var(--radius-button);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
    font-size: var(--text-md);
    transition:
      background-color var(--duration-micro) var(--ease-out),
      transform var(--duration-micro) var(--ease-out);
  }
  .key:hover {
    background: color-mix(in srgb, var(--foreground) 14%, transparent);
  }
  .key:active {
    transform: scale(0.94);
  }
  .key.active {
    background: var(--foreground);
    color: var(--color-fg-inverse);
  }
  .wide {
    min-width: 3.25rem;
  }
  .space {
    flex: 1;
    max-width: 22rem;
  }
  .enter {
    min-width: 6rem;
    background: color-mix(in srgb, var(--foreground) 90%, transparent);
    color: var(--color-fg-inverse);
    font-size: var(--text-base);
  }
  .enter:hover {
    background: var(--foreground);
  }
  :global([data-contrast="high"]) .osk {
    background: #000000;
    border-color: #ffffff;
  }
  :global([data-contrast="high"]) .key {
    background: #000000;
    border-color: #ffffff;
  }
</style>
