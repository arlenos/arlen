<script lang="ts">
  /// The active prompt line at the foot of the block stream
  /// (terminal.md §4.3, corrected 11 June): the input IS the next
  /// line in the stream — the same PromptLine chrome a finished
  /// block has, the prompt char, and a bare inline input. No bordered
  /// textarea, no pinned bar, no ornament. Enter runs the line; a
  /// history pick lands here via the prefill store. Single-line by
  /// design (the multiline costume is M4).
  import { tick } from "svelte";
  import { writable } from "svelte/store";
  import { terminalInput, type GitInfo, type Session } from "$lib/contract";
  import { composerPrefill } from "$lib/stores/composer";
  import PromptLine from "./PromptLine.svelte";

  let {
    session,
    git = null,
    onsent,
  }: {
    /// The active session; null hides the prompt entirely (the page
    /// shows its failure state instead). An exited session shows the
    /// line disabled with the restart hint as its placeholder.
    session: Session | null;
    /// Git state for the live prompt — the host hands down the last
    /// block's (live git belongs to the engine seam, flagged).
    git?: GitInfo | null;
    /// Called after the backend accepted the input, so the page can
    /// refresh the block stream.
    onsent?: () => void;
  } = $props();

  let draft = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  const busy = writable(false);

  const usable = $derived(session !== null && session.status === "running");
  const placeholder = $derived(
    session?.status === "exited" ? "Session ended. Ctrl+T starts a new one." : "",
  );

  // Take a pending prefill (a history pick) as the draft.
  $effect(() => {
    const text = $composerPrefill;
    if (text !== null) {
      draft = text;
      composerPrefill.set(null);
      inputRef?.focus();
    }
  });

  // Focus follows the active session, without stealing: only when
  // nothing else holds focus (the Ctrl+R palette keeps its claim).
  let focusedSession: string | null = null;
  $effect(() => {
    const id = usable ? (session?.id ?? null) : null;
    if (id === focusedSession) return;
    focusedSession = id;
    if (id === null) return;
    tick().then(() => {
      const ae = document.activeElement;
      if (!ae || ae === document.body || ae === inputRef) {
        inputRef?.focus();
      }
    });
  });

  async function submit() {
    const text = draft;
    if (!text.trim() || $busy || !session) return;
    busy.set(true);
    try {
      // Send the line WITH a trailing newline: the newline is the Enter
      // the shell needs to actually run the command. Without it the PTY
      // only buffers the characters and nothing ever executes - which is
      // why the terminal showed no command output.
      await terminalInput(session.id, text + "\n");
      draft = "";
      onsent?.();
    } catch {
      // The shell did not accept the input; the draft stays put.
    }
    busy.set(false);
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    }
  }

  /// Typing while scrolled up jumps the view back to the prompt, the
  /// way every terminal returns to the tape end on input.
  function onInput() {
    inputRef?.scrollIntoView({ block: "nearest" });
  }
</script>

{#if session}
  <div class="active-prompt">
    <PromptLine cwd={session.cwd} {git} />
    <div class="ap-line">
      <span class="ap-char" aria-hidden="true">❯</span>
      <input
        id="terminal-composer-input"
        bind:this={inputRef}
        bind:value={draft}
        class="ap-input"
        type="text"
        autocomplete="off"
        autocapitalize="off"
        spellcheck="false"
        {placeholder}
        disabled={!usable || $busy}
        aria-label="Command input"
        onkeydown={onKeydown}
        oninput={onInput}
      />
    </div>
  </div>
{/if}

<style>
  /* A stream row like any block: same horizontal edge, no box. */
  .active-prompt {
    flex-shrink: 0;
    padding: 12px 16px;
  }

  .ap-line {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  /* Full strength: this is THE input spot; the resting block
     markers stay dimmed. Chevron from plain JetBrains Mono (the NF
     Mono variant squeezes it). */
  .ap-char {
    flex-shrink: 0;
    font-family: "JetBrains Mono", var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
  }
  /* The input is bare text in the stream: no border, no background,
     no focus ring — the blinking caret is the affordance. */
  .ap-input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    padding: 0;
    color: var(--foreground);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
    outline: none;
  }
  .ap-input::placeholder {
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .ap-input:disabled {
    opacity: 1;
  }
</style>
