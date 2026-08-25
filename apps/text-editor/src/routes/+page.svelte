<script lang="ts">
  import { printProblem } from "$lib/printProblem";
  /// The editor window: a two-pane surface - the text canvas + the KG-lens panel.
  /// The lens is a co-star (it is the reason the editor exists), not a hidden
  /// sidebar. The slim titlebar carries the file name, a focus-mode toggle, and the
  /// as-of scrubber (time-travel over the file + its context).
  import { invoke } from "@tauri-apps/api/core";
  import Buffer from "$lib/components/editor/Buffer.svelte";
  import Canvas from "$lib/components/editor/Canvas.svelte";
  import LensPanel from "$lib/components/editor/LensPanel.svelte";
  import AiEditReview from "$lib/components/editor/AiEditReview.svelte";
  import { loadLens } from "$lib/stores/lens";
  import { openDocument, openError, openTarget, loadInitialFile, saveProblemKey } from "$lib/stores/document";
  import { onMount } from "svelte";
  import { initAppMenu, menuAction } from "$lib/menu";
  import { proposal, proposeEdit, dismiss } from "$lib/stores/aiEdit";
  import { t, dir } from "$lib/i18n/messages";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Sun, PanelRight, Hash, Printer } from "lucide-svelte";

  // The AI edit is invoked by keyboard (Cmd/Ctrl+K), never a bolted-on titlebar
  // button. Its discoverable home is a future command palette; a text-selection
  // "edit this" action is the contextual one. Escape dismisses an open proposal.
  function onKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      e.preventDefault();
      if (!$proposal) void proposeEdit("Tighten the intro and add a reference");
    } else if (e.key === "Escape" && $proposal) {
      e.preventDefault();
      dismiss();
    }
  }

  // The transaction-time presets (mirrors apps/files/src/lib/asof.ts). Derived so the
  // labels re-resolve when the locale switches.
  const AS_OF_OPTIONS = $derived([
    { value: "now", label: $t("te.asOf.now") },
    { value: "1d", label: $t("te.asOf.1d") },
    { value: "1w", label: $t("te.asOf.1w") },
    { value: "1m", label: $t("te.asOf.1m") },
  ]);

  /// The selected preset as an instant in epoch MICROSECONDS, or null for "now",
  /// which keeps the live read rather than asking for the present as a past time.
  function asOfMicros(preset: string): number | null {
    const DAY = 86_400_000;
    const back = preset === "1d" ? DAY : preset === "1w" ? 7 * DAY : preset === "1m" ? 30 * DAY : 0;
    return back === 0 ? null : (Date.now() - back) * 1000;
  }

  let focusMode = $state(false);
  let lensOpen = $state(true);
  let asOf = $state("now");
  let fileIdx = $state(0);
  let lineNumbers = $state(true);

  const MD_DOC = `# The KG-lens

This file is a **first-class citizen** of the knowledge graph. Beside the text, Arlen surfaces where it came from, the notes that mention it, and the project it belongs to.

## Why not gedit

A plain editor is a solved category. The reason to build our own is the lens and the [gated AI-edit](lens-design.md): the assistant is a bounded, auditable, reversible principal that can edit this file.

## The gate, in code

Before the assistant writes, its edit is authorized:

\`\`\`ts
type AuthorizeDecision =
  | { decision: "allow" }                     // reversible, autonomous
  | { decision: "confirm"; prompt: string }   // irreversible, ask first
  | { decision: "deny"; reason: string };
\`\`\`

## Focus mode

Turn this on and every paragraph but the one you are in fades away, so the writing is all that is left. The markdown you see is the real \`bytes\` on disk, never hidden.`;

  const CODE_DOC = `// The Arlen gate: every AI tool call is authorized before it runs.
import { invoke } from "@tauri-apps/api/core";

export type AuthorizeDecision =
  | { decision: "allow"; proof?: string }
  | { decision: "confirm"; prompt: string }
  | { decision: "deny"; reason: string };

// Reversible edits run autonomously; irreversible ones are held for the user.
export async function authorize(call: ToolCall): Promise<AuthorizeDecision> {
  const verdict = await invoke("Authorize", { call });
  if (verdict.decision === "deny") {
    return { decision: "deny", reason: verdict.reason };
  }
  return verdict;
}`;

  // The two demo documents, shown when the editor is launched with no file. They
  // describe the editor itself, so they claim nothing about the user's machine.
  const FILES = [
    { name: "the-kg-lens.md", type: "markdown" as const, content: MD_DOC },
    { name: "gate.ts", type: "code" as const, content: CODE_DOC },
  ];

  // A real launch file wins over the demos, and replaces the picker with its own
  // name: offering to switch back to a demo document from a file the user opened
  // would put invented text one click from their own.
  const file = $derived($openDocument ?? FILES[fileIdx]);

  /// The buffer's text, and whether it differs from what is on disk.
  ///
  /// Only a REAL file gets the editable buffer: the two demo documents are shown
  /// through the reading canvas, because offering to save invented text under an
  /// invented name would be the one thing this app must never do.
  let draft = $state<string | null>(null);
  let saveError = $state<string | null>(null);
  /// The file was written by something else since it was opened. A question,
  /// not a failure.
  let changedOnDisk = $state(false);
  let savedAt = $state(0);
  const editable = $derived(!!$openDocument);
  const dirty = $derived(draft !== null && draft !== file.content);

  /// The grammar for the open file, from its extension. Unknown means no
  /// highlighting rather than a guess.
  const language = $derived.by(() => {
    const name = ($openDocument?.name ?? "").toLowerCase();
    if (name.endsWith(".md") || name.endsWith(".markdown")) return "markdown" as const;
    if (name.endsWith(".rs")) return "rust" as const;
    if (name.endsWith(".js") || name.endsWith(".ts") || name.endsWith(".json")) return "javascript" as const;
    return "text" as const;
  });

  /// Write the buffer back through the host, which does the temp-file-and-rename.
  ///
  /// A failure is SHOWN. An editor that silently fails to save is worse than one
  /// that cannot save at all, because the user walks away believing their work is
  /// on disk.
  async function save(force = false) {
    const target = $openDocument;
    if (!target || draft === null) return;
    saveError = null;
    changedOnDisk = false;
    try {
      // The stamp goes back with the text: if the file no longer matches it,
      // something else has written it since this was opened and the host refuses
      // rather than destroying that silently.
      const stamp = await invoke<string>("editor_save", {
        path: target.path,
        text: draft,
        seen: target.stamp,
        force,
      });
      // The document is now what is on disk, so the buffer is no longer dirty
      // without having to re-read the file - and the new stamp is what the NEXT
      // save compares against.
      openDocument.set({ ...target, content: draft, stamp });
      savedAt = Date.now();
    } catch (e) {
      // Its own state, not an error string: this is a question for the person
      // rather than a failure, and it has an answer they can give.
      // Still a substring test, and deliberately: the host answers this one with
      // the tag `file-changed-on-disk`, and a Tauri error arrives here either as
      // the object or as a string with the JSON inside it depending on the path.
      if (String(e).includes("file-changed-on-disk")) changedOnDisk = true;
      else saveError = saveProblemKey(e);
    }
  }
  /// What the print portal last said, so the person is told rather than left
  /// guessing whether anything happened.
  let printStatus = $state<string | null>(null);

  /// Hand the open file to the print portal.
  ///
  /// The FILE on disk, not the buffer: an unsaved change is not in the file the
  /// portal reads, and printing a version the person cannot see would be a
  /// quieter lie than refusing. The status below says which state they are in.
  async function print() {
    const target = $openDocument;
    if (!target) return;
    printStatus = $t("te.print.pending");
    try {
      const r = await invoke<{ outcome: string }>("plugin:arlen-shell|print_file", {
        path: target.path,
      });
      printStatus =
        r.outcome === "sent"
          ? $t("te.print.sent")
          : r.outcome === "cancelled"
            ? $t("te.print.cancelled")
            : r.outcome === "refused"
              ? $t("te.print.refused")
              : $t("te.print.noAnswer");
    } catch (e) {
      const p = printProblem(String(e));
      printStatus =
        p.key === "te.print.noPortal"
          ? $t("te.print.noPortal")
          : p.key === "te.print.noBus"
            ? $t("te.print.noBus")
            : p.key === "te.print.fileUnreadable"
              ? $t("te.print.fileUnreadable", { message: p.detail })
              : $t("te.print.failed", { reason: p.detail });
    }
  }
  // A launch file names the window even when it failed to open: the alternative
  // is a demo document's name over a pane that says the file could not be read.
  const fileOptions = $derived(
    $openTarget
      ? [{ value: "0", label: $openDocument?.name ?? $openTarget }]
      : FILES.map((f, i) => ({ value: String(i), label: f.name })),
  );

  // The shell menu's dispatch: the same verbs the keys and buttons run.
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "file.save") void save();
    else if (a === "file.print") void print();
  });

  onMount(() => {
    void initAppMenu();
    void loadInitialFile();
  });

  // The lens tracks whichever file is open, and is given the PATH when there is
  // one. A basename is ambiguous and the lens resolves it as a trailing segment,
  // so two projects each holding a `README.md` make the panel name whichever the
  // graph returned first: opening `atlas/README.md` said "Part of beacon", which
  // is a confident false claim about the open file. The demo documents have no
  // path and keep their name.
  $effect(() => {
    loadLens($openDocument?.path ?? file.name, asOfMicros(asOf));
  });

  // Window chrome: the toolbar doubles as the drag region (explicit
  // startDragging - the drag attribute is unreliable on Wayland in Tauri v2),
  // guarded so vite still renders.
  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }
  async function startDrag(e: PointerEvent): Promise<void> {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      // No Tauri runtime under vite: the toolbar is a static bar.
    }
  }
  async function toggleMax(e: MouseEvent): Promise<void> {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      // Same guard as above.
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="app" dir={$dir}>
  <!-- The toolbar is a drag surface (a non-keyboard pointer interaction); its
       actual controls are accessible buttons inside it, so the
       static-interaction lint is a false positive here. Same treatment as the
       knowledge and store headers. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="titlebar" onpointerdown={startDrag} ondblclick={toggleMax}>
    <PopoverSelect
      value={$openTarget ? "0" : String(fileIdx)}
      options={fileOptions}
      width="170px"
      ariaLabel={$t("te.openFile")}
      onchange={(v) => (fileIdx = Number(v))}
    />
    <!-- THE SAVE STATE, RENDERED. It was computed and not shown until 16 August:
         `dirty`, `savedAt` and `saveError` were all assigned by the save path and
         reached no markup, so a failed write left the file unsaved in silence -
         the exact defect this file's own comment calls worse than not saving at
         all. Written by me an hour before it was found, which is the argument for
         driving a surface rather than reading it. -->
    {#if editable}
      <span class="savestate" aria-live="polite">
        {#if printStatus}
          <!-- Not `ss-bad`: a cancelled print, or a dialog still open, is not a
               failure and must not be coloured as one. -->
          <span class="ss-ok" role="status">{printStatus}</span>
        {:else if saveError}
          <span class="ss-bad" role="alert">{$t(saveError)}</span>
        {:else if dirty}
          <span class="ss-dirty">{$t("te.save.unsaved")}</span>
        {:else if savedAt}
          <span class="ss-ok">{$t("te.save.saved")}</span>
        {/if}
      </span>
    {/if}
    <span class="spacer"></span>
    {#if file.type === "code"}
      <IconAction
        label={$t("te.lineNumbers.toggle")}
        size="control"
        active={lineNumbers}
        onclick={() => (lineNumbers = !lineNumbers)}
      >
        <Hash size={15} strokeWidth={1.75} />
      </IconAction>
    {:else}
      <Button variant={focusMode ? "default" : "outline"} size="sm" onclick={() => (focusMode = !focusMode)}>
        <Sun size={14} strokeWidth={2} /> {$t("te.focus")}
      </Button>
    {/if}
    <!-- Live: picking a past instant re-reads the lens as of then. It was
         disabled while promotion wrote no interval stamps, and again while only
         the query existed; both halves are built now. An instant before the graph
         began recording is answered as "not recorded", never as "no project" -
         those are opposite claims and the panel keeps them apart. -->
    <PopoverSelect
      value={asOf}
      options={AS_OF_OPTIONS}
      width="130px"
      ariaLabel={$t("te.asOf.aria")}
      onchange={(v) => (asOf = v)}
    />
    {#if editable}
      <IconAction label={$t("te.print")} size="control" onclick={() => print()}>
        <Printer size={15} strokeWidth={1.75} />
      </IconAction>
    {/if}
    <IconAction
      label={$t("te.lens.toggle")}
      size="control"
      active={lensOpen}
      onclick={() => (lensOpen = !lensOpen)}
    >
      <PanelRight size={15} strokeWidth={1.75} />
    </IconAction>
    <WindowButtons />
  </header>

  {#if changedOnDisk}
    <div class="disk-bar" role="alert">
      <span>{$t("te.save.changedOnDisk")}</span>
      <button type="button" onclick={() => save(true)}>{$t("te.save.overwrite")}</button>
      <button type="button" class="quiet" onclick={() => (changedOnDisk = false)}>
        {$t("te.save.keepEditing")}
      </button>
    </div>
  {/if}

  <div class="body">
    <main class="editor">
      {#if $openError}
        <!-- The editor was asked to open a file and could not. The host's message
             names the path and the reason; showing anything else here would mean
             putting text on screen under a filename that is not its text. -->
        <div class="open-failed" role="alert">
          <p class="of-title">{$t("te.open.failed")}</p>
          <p class="of-detail">
            {#if $openError.problem === "not-absolute"}{$t("te.open.notAbsolute")}
            {:else if $openError.problem === "not-text"}{$t("te.open.notText")}
            {:else if $openError.problem === "unreadable"}{$t("te.open.unreadable", { why: $openError.why })}
            {:else}{$t("te.open.otherReason")}{/if}
          </p>
        </div>
      {:else if editable}
        <!-- A real file gets the real buffer. The demo documents below keep the
             reading canvas: they are not on disk, and an editor that let you type
             into invented text under an invented name would be inviting work that
             cannot be saved anywhere. -->
        <Buffer
          doc={file.content}
          {language}
          onchange={(t) => (draft = t)}
          onsave={() => void save()}
        />
      {:else}
        <!-- SAY IT IS A SAMPLE. The picker shows `the-kg-lens.md` and the lens
             beside it answers real queries about a file of that name, which finds
             nothing - so without this line the window reads as a document on this
             machine that the graph happens to know nothing about. It is not on the
             machine at all. The app already says this about a sample lens and a
             sample proposal; the document it is written about deserves the same
             sentence. -->
        <p class="demo-note">{$t("te.demoDoc")}</p>
        <Canvas doc={file.content} fileType={file.type} {focusMode} {lineNumbers} />
      {/if}
    </main>
    {#if $proposal}
      <AiEditReview />
    {:else if lensOpen}
      <LensPanel />
    {/if}
  </div>
</div>

<style>
  /* The changed-on-disk bar: a question, so it sits across the width where the
     whole sentence fits, rather than in the toolbar strip that truncates it. */
  .disk-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    font-size: 12px;
    background: color-mix(in srgb, var(--color-fg-warning, #eab308) 12%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-warning, #eab308) 35%, transparent);
    color: var(--color-fg-primary, #fafafa);
  }
  .disk-bar span {
    flex: 1;
  }
  .disk-bar button {
    border: 1px solid var(--color-border-default, #333);
    background: var(--color-bg-card, #171717);
    color: inherit;
    border-radius: 5px;
    padding: 3px 9px;
    font: inherit;
    cursor: pointer;
  }
  .disk-bar button.quiet {
    background: transparent;
  }


  .demo-note {
    margin: 0 0 var(--space-3);
    color: var(--color-fg-secondary);
    font-size: var(--text-sm);
  }
  .open-failed {
    padding: 2.5rem 2rem;
    max-width: 34rem;
  }
  .of-title {
    margin: 0 0 0.4rem;
    font-size: 0.95rem;
    font-weight: 600;
  }
  .of-detail {
    margin: 0;
    font-size: 0.85rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 62%, transparent);
    word-break: break-word;
  }
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
  }
  .savestate {
    font-size: 12px;
    margin-inline-start: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 40ch;
  }
  .ss-dirty {
    color: color-mix(in srgb, var(--color-fg-primary, #fafafa) 55%, transparent);
  }
  .ss-ok {
    color: color-mix(in srgb, var(--color-fg-primary, #fafafa) 40%, transparent);
  }
  .ss-bad {
    color: var(--color-error, #ef4444);
  }
  .titlebar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    height: 2.5rem;
    padding: 0 1rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    flex-shrink: 0;
  }
  .spacer {
    flex: 1;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  .editor {
    flex: 1;
    overflow-y: auto;
    padding: 1.5rem 2rem;
  }
</style>
