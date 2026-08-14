<script lang="ts">
  /// NOT MOUNTED. Nothing renders this since xterm.js took the whole terminal
  /// (the DOM block stream went with it, having double-rendered against the
  /// grid). Its refusal line therefore ships nowhere, and I built that line here
  /// before checking - a fix wired in code and invisible in deployment, which is
  /// the defect this file's own comments are about, committed by the person
  /// clearing it. Verified in a harness that mounted the component directly,
  /// which proves a component works and says nothing about whether anyone sees
  /// it.
  ///
  /// The live equivalent is in `Terminal.svelte`: a refused write says so as a
  /// line in the grid, which is where a terminal says things.
  ///
  /// Kept rather than deleted because whether the prompt row returns is a design
  /// question, not a cleanup. If it does, this is the shape it had.
  ///
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
  import { t } from "$lib/i18n/messages";
  import { composerPrefill } from "$lib/stores/composer";
  import { newSessionFailed } from "$lib/stores/sessions";
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
  /// True when the last Enter was refused by the shell. Keeping the draft is
  /// right - losing what someone typed is worse - but keeping it SILENTLY means
  /// Enter did nothing and said nothing, which reads as a dead terminal rather
  /// than a refused line. Cleared the moment they type again, because by then
  /// they are already acting on it.
  let notAccepted = $state(false);

  /// The one refusal line for this row. The input's own refusal wins when both
  /// are set, because it is the thing they just did.
  ///
  /// A session refusal reaches here rather than dying in a `catch`: four of the
  /// five `newSession()` call sites fire while a working terminal is on screen -
  /// the sidebar menu, the topbar, Ctrl+T, the auto-create - and the stranded
  /// panel that carries the other one is not rendered then. Four sibling apps
  /// already keep a last-refusal store and render it as an alert line; this is
  /// the terminal's, in the row where its actions happen.
  ///
  /// A different sentence from the stranded panel's for the same fact, which is
  /// the rule rather than an exception: there the title already says a session
  /// could not start, so the line adds only a cause; here it stands alone and has
  /// to name what failed before naming why.
  const refusal = $derived(
    notAccepted ? $t("term.notAccepted") : $newSessionFailed ? $t("term.err.newSessionRefused") : null,
  );
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
    notAccepted = false;
    try {
      // Send the line WITH a trailing newline: the newline is the Enter
      // the shell needs to actually run the command. Without it the PTY
      // only buffers the characters and nothing ever executes - which is
      // why the terminal showed no command output.
      await terminalInput(session.id, text + "\n");
      draft = "";
      onsent?.();
    } catch {
      // The shell did not accept the input. The draft stays put AND the line
      // below says so: a refusal nobody is told about is the same as a key that
      // does nothing.
      notAccepted = true;
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
    notAccepted = false;
    newSessionFailed.set(false);
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
        aria-label={$t("term.commandInput")}
        onkeydown={onKeydown}
        oninput={onInput}
      />
    </div>
    {#if refusal}
      <p class="ap-refused" role="alert">{refusal}</p>
    {/if}
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
  /* Under the line rather than over it: the prompt keeps its place in the
     stream, and the sentence reads as a note about the line just typed. */
  .ap-refused {
    margin: 4px 0 0;
    padding-left: 20px;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-error, #f87171) 85%, var(--foreground));
  }
  /* Full strength: this is THE input spot; the resting block
     markers stay dimmed. Chevron from plain JetBrains Mono (the NF
     Mono variant squeezes it). */
  .ap-char {
    flex-shrink: 0;
    font-family: "JetBrains Mono", var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-sm);
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
    font-size: var(--text-sm);
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
