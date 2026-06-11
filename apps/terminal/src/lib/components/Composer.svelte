<script lang="ts">
  /// The command composer pinned under the stream: a prompt glyph, a
  /// growing mono textarea and the capability strip below. Enter runs
  /// the command through `terminal_input`, Shift+Enter breaks the
  /// line. A history row click lands here via the prefill store; a
  /// new active session pulls focus here so typing starts without a
  /// click — unless something else (the history search) already
  /// holds it.
  import { tick } from "svelte";
  import { writable } from "svelte/store";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { terminalInput, type Session } from "$lib/contract";
  import { composerPrefill } from "$lib/stores/composer";
  import CapabilityIndicator from "./CapabilityIndicator.svelte";

  let {
    session,
    onsent,
  }: {
    /// The active session; null disables the composer. An exited
    /// session also disables it — input into a dead shell goes
    /// nowhere, the placeholder says how to get a live one.
    session: Session | null;
    /// Called after the backend accepted the input, so the page can
    /// refresh the block stream.
    onsent?: () => void;
  } = $props();

  let draft = $state("");
  let textareaRef = $state<HTMLTextAreaElement | null>(null);
  const busy = writable(false);

  const usable = $derived(session !== null && session.status === "running");
  const placeholder = $derived(
    session === null
      ? "Open a session to run commands"
      : session.status === "exited"
        ? "Session ended. Ctrl+T starts a new one."
        : "Run a command",
  );

  // Take a pending prefill (a history row click) as the draft.
  $effect(() => {
    const text = $composerPrefill;
    if (text !== null) {
      draft = text;
      composerPrefill.set(null);
      textareaRef?.focus();
    }
  });

  // Focus follows the active session, without stealing: only when
  // nothing else holds focus (the Ctrl+R search keeps its claim).
  let focusedSession: string | null = null;
  $effect(() => {
    const id = usable ? (session?.id ?? null) : null;
    if (id === focusedSession) return;
    focusedSession = id;
    if (id === null) return;
    tick().then(() => {
      const ae = document.activeElement;
      if (!ae || ae === document.body || ae === textareaRef) {
        textareaRef?.focus();
      }
    });
  });

  async function submit() {
    const text = draft;
    if (!text.trim() || $busy || !usable || !session) return;
    busy.set(true);
    try {
      await terminalInput(session.id, text);
      draft = "";
      onsent?.();
    } catch {
      // The shell did not accept the input; the draft stays put.
    }
    busy.set(false);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }
</script>

<div class="composer-zone">
  <div class="composer" class:disabled={!usable}>
    <span class="prompt-glyph" aria-hidden="true">❯</span>
    <Textarea
      id="terminal-composer-input"
      bind:ref={textareaRef}
      bind:value={draft}
      rows={1}
      maxRows={6}
      class="composer-input"
      {placeholder}
      disabled={!usable || $busy}
      aria-label="Command input"
      onkeydown={onKeydown}
    />
    <CapabilityIndicator />
  </div>
</div>

<style>
  .composer-zone {
    flex-shrink: 0;
    padding: 12px 16px;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }

  .composer {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 12px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--color-bg-input, var(--background));
    transition: border-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .composer:focus-within {
    border-color: var(--control-border-hover);
  }
  .composer.disabled {
    opacity: 0.7;
  }

  /* Quiet prompt char: accent at the marker slot now means "the
     agent ran this", so the composer's char stays neutral and the
     focus ring carries "type here". */
  .prompt-glyph {
    flex-shrink: 0;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.25rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* The textarea is borderless inside the container; the container is
     the control. */
  .composer :global(.composer-input) {
    border: none;
    background: transparent;
    padding: 0;
    border-radius: 0;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
  }
  .composer :global(.composer-input:focus-visible) {
    outline: none;
    box-shadow: none;
    --tw-ring-color: transparent;
  }
</style>
