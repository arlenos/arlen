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
  import { publishPresence, recordSave } from "$lib/graphInput";
  import { proposal, proposeEdit, dismiss } from "$lib/stores/aiEdit";
  import { t, dir } from "$lib/i18n/messages";
  import { kt } from "@arlen/ui-kit/i18n/messages.kit";
  import { ioWhyKey } from "@arlen/ui-kit/io-why";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
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
      // The moment the work landed, on the timeline. HERE and not beside the
      // press: a save that the host refused is not a moment, and the graph is
      // meant to hold what happened rather than what was attempted.
      void recordSave(target.path, draft.length, language);
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
  /// The print's outcome is a value for the title bar's slot; a print that
  /// did not happen is a refusal, and refusals have one shape, the Notice at
  /// the top of the editor (design-system.md 6.11, thread two).
  let printOutcome = $state<string | null>(null);
  let printFailure = $state<string | null>(null);

  /// Hand the open file to the print portal.
  ///
  /// The FILE on disk, not the buffer: an unsaved change is not in the file the
  /// portal reads, and printing a version the person cannot see would be a
  /// quieter lie than refusing. The status below says which state they are in.
  async function print() {
    const target = $openDocument;
    if (!target) return;
    printOutcome = $t("te.print.pending");
    printFailure = null;
    try {
      const r = await invoke<{ outcome: string }>("plugin:arlen-shell|print_file", {
        path: target.path,
      });
      if (r.outcome === "sent") printOutcome = $t("te.print.sent");
      else if (r.outcome === "cancelled") printOutcome = $t("te.print.cancelled");
      else {
        printOutcome = null;
        printFailure = r.outcome === "refused" ? $t("te.print.refused") : $t("te.print.noAnswer");
      }
    } catch (e) {
      const p = printProblem(String(e));
      printOutcome = null;
      printFailure =
        p.key === "te.print.noPortal"
          ? $t("te.print.noPortal")
          : p.key === "te.print.noBus"
            ? $t("te.print.noBus")
            : p.key === "te.print.refused"
              ? $t("te.print.refused")
            : p.key === "te.print.fileUnreadable"
              ? $t("te.print.fileUnreadable")
              : $t("te.print.failed");
      // The plugin's own text is for whoever reads the console, never the page.
      if (p.key === "te.print.failed") console.warn("text-editor: the print did not start", p.detail);
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
    // PRESENCE IS EPHEMERAL, so somebody has to end it. The SDK emits and leaves
    // the WHEN to the app; for an editor it is the window losing focus, after
    // which "currently editing this" is a claim nobody can stand behind. The
    // payload carries the same intent as a hint, so a future shell-side
    // auto-clear and this agree instead of racing.
    let stop: (() => void) | null = null;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void publishPresence($openDocument?.path ?? null, language);
        else void publishPresence(null, language);
      })
      .then((un) => {
        stop = un;
      })
      .catch(() => {
        // No toplevel (vite): nothing to lose focus, so nothing to clear.
      });
    return () => stop?.();
  });

  /// What this window is doing, while it is doing it. Re-published when the file
  /// or its language changes; the editor is the only thing that knows a path is
  /// being EDITED rather than read, which is the whole reason this surface
  /// exists.
  $effect(() => {
    void publishPresence($openDocument?.path ?? null, language);
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

  /// The host's errno text as a sentence of the reader's, or nothing.
  /// The open failure as one finished text. Three of the causes are whole
  /// sentences on their own; only the unnamed one needs "could not be opened"
  /// in front of it.
  const openText = $derived.by(() => {
    const e = $openError;
    if (!e) return "";
    if (e.problem === "not-absolute") return $t("te.open.notAbsolute");
    if (e.problem === "not-text") return $t("te.open.notText");
    if (e.problem === "unreadable") return $t("te.open.unreadable", { why: whyText(e.why) });
    return `${$t("te.open.failed")} ${$t("te.open.otherReason")}`;
  });

  const whyText = (text: string): string => {
    const key = ioWhyKey(text);
    return key ? $kt(key) : "";
  };
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
        {#if printOutcome}
          <!-- A cancelled print, or a dialog still open, is an outcome rather
               than a failure; the failures are Notices over the editor. -->
          <span class="ss-ok" role="status">{printOutcome}</span>
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

  <div class="body">
    <main class="editor">
      <!-- The page's one level-one heading. Every app in this tree had none, so a
           screen reader's first question - what IS this window - was answered only
           by the window title, which is not in the document. The app NAME rather
           than the visible bar title, which says where you are inside the app and
           changes as you move; hidden, because the bar already shows that and a
           second visible title would be the same fact twice. -->
      <h1 class="sr-only">{$t("te.app.title")}</h1>
      <!-- Every refusal of this surface, in one shape at its top (design-system.md
           6.11, thread two): the file that could not be opened, the save that
           was refused, the print that did not happen, and the one question with
           an answer, a file that changed on disk. -->
      {#if $openError}
        <!-- The editor was asked to open a file and could not; nothing goes on
             the canvas under a filename that is not its text. -->
        <div class="note"><Notice tone="error" text={openText} /></div>
      {/if}
      {#if saveError}
        <div class="note"><Notice tone="error" text={$t(saveError)} /></div>
      {/if}
      {#if printFailure}
        <div class="note"><Notice tone="error" text={printFailure} /></div>
      {/if}
      {#if changedOnDisk}
        <div class="note note-row">
          <Notice tone="caution" text={$t("te.save.changedOnDisk")} />
          <Button size="sm" onclick={() => save(true)}>{$t("te.save.overwrite")}</Button>
          <Button variant="ghost" size="sm" onclick={() => (changedOnDisk = false)}>{$t("te.save.keepEditing")}</Button>
        </div>
      {/if}
      {#if $openError}
        <!-- The canvas stays empty. -->
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
        <div class="note"><Notice tone="neutral" text={$t("te.demoDoc")} /></div>
        <!-- The canvas is plain prose with no scroller of its own, so the box
             that holds it is the one that scrolls. -->
        <div class="reading">
          <Canvas doc={file.content} fileType={file.type} {focusMode} {lineNumbers} />
        </div>
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

  .note {
    margin: 0 0 var(--space-3);
  }
  /* A refusal with an answer: the Notice takes the line, the buttons follow. */
  .note-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }
  .note-row :global(.notice) {
    flex: 1;
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
  /* The cap is right for a status and wrong for a refusal, so the refusal lifts
     it and wraps instead. It keeps the ellipsis machinery off rather than
     widening the cap, because no number is wide enough for every language. */
  .ss-dirty {
    color: color-mix(in srgb, var(--color-fg-primary, #fafafa) 55%, transparent);
  }
  .ss-ok {
    color: color-mix(in srgb, var(--color-fg-primary, #fafafa) 40%, transparent);
  }
  /* `min-height` and wrap, for the one state on this bar that is a sentence.
     Everything else it shows is two words - "Gespeichert", "Nicht gespeichert" -
     and a refusal is not: "Konnte nicht speichern: diese Datei oder ihr Ordner
     ließ sich nicht beschreiben." Measured under the refuse-save fixture, the
     bar rendered "Konnte nicht speichern: diese Da…" at 720, 1280 AND 1920,
     because a 40ch cap does not care how wide the window is. What was cut is the
     CAUSE - the half a refusal exists to give. */
  .titlebar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
    min-height: 2.5rem;
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
  /* THE SURFACE DOES NOT SCROLL; WHAT IS IN IT DOES. It used to carry
     `overflow-y: auto` over a buffer that is `height: 100%`, so the moment a
     refusal notice appeared above the buffer the whole surface grew past its
     box by exactly the notice's height and started scrolling - two nested
     scrollers fighting, and the refusal scrolling away from the thing it was
     about. axe rates it serious for a second reason: the outer region scrolls
     and, under this engine, nothing inside it reports as tabbable, so a
     keyboard has no way to reach what the scroll hides.

     A column now: the notices take their height, the reading or editing
     surface takes the rest and owns its own scrolling. CodeMirror already
     brings one; the demo canvas is plain content, so it gets the `reading`
     box. */
  .editor {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding: 1.5rem 2rem;
  }
  .editor > :global(.buffer) {
    flex: 1;
    min-height: 0;
    height: auto;
  }
  .reading {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
