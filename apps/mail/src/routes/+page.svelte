<script lang="ts">
  /// The mail client, three panes on the files chrome: the folder rail, the
  /// message list, the reading surface. The mailbox model is the intended
  /// contract (mailbox.ts) - fixture under vite, honestly unconnected on a
  /// host without the account backend - while the one wire that IS real today
  /// (`launch_file` + `mail_read`, a message opened from Files) renders into
  /// the same reading surface as a transient row that belongs to no folder.
  ///
  /// The HTML part of a message stays deliberately absent - see the app's
  /// `lib.rs` (EFAIL): containing the renderer does not stop the message
  /// calling home. The reading surface states that as a fact, not an apology.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { Mail, Reply, Forward, Archive, Trash2, FileText } from "@lucide/svelte";
  import { t } from "$lib/i18n/messages";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import FolderRail from "$lib/components/FolderRail.svelte";
  import MessageList from "$lib/components/MessageList.svelte";
  import MessageView from "$lib/components/MessageView.svelte";
  import ComposeView from "$lib/components/ComposeView.svelte";
  import {
    folders,
    envelopes,
    mailboxMocked,
    openedFile,
    loadMailbox,
    openMessage,
    markRead,
    moveMessage,
    deleteForever,
    type FolderKind,
    type Message,
  } from "$lib/stores/mailbox";

  /// Whether there is a host to ask at all. In a browser there is none, and
  /// that is not a failure to report.
  const tauriAvailable = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

  let selectedFolder = $state("inbox");
  let selectedId = $state<string | null>(null);
  let reading = $state<Message | null>(null);
  let composing = $state(false);
  let preset = $state<{ to: string; subject: string; body: string }>({ to: "", subject: "", body: "" });

  type Failure =
    | { problem: "unreadable"; why: string }
    | { problem: "not-a-message" }
    | { problem: "other"; reason: string };
  let failure = $state<Failure | null>(null);

  onMount(() => {
    void loadMailbox();
    if (!tauriAvailable) return;
    void (async () => {
      const launched = await invoke<string | null>("launch_file").catch(() => null);
      if (!launched) return;
      try {
        const m = await invoke<Message>("mail_read", { path: launched });
        openedFile.set(m);
        selectedId = "@file";
        failure = null;
      } catch (e) {
        // The payload arrives as an OBJECT here, not as a string with JSON
        // inside it - measured on the wine-manager window, which printed
        // `[object Object]` until this accepted both shapes. `apps/viewers`
        // documents the string form and is right about its own case.
        const named =
          e && typeof e === "object"
            ? (e as Record<string, unknown>)
            : (() => {
                const raw = String(e);
                const at = raw.indexOf("{");
                try {
                  return at >= 0 ? (JSON.parse(raw.slice(at)) as Record<string, unknown>) : null;
                } catch {
                  return null;
                }
              })();
        if (named?.problem === "unreadable")
          failure = { problem: "unreadable", why: String(named.why ?? "") };
        else if (named?.problem === "not-a-message") failure = { problem: "not-a-message" };
        else failure = { problem: "other", reason: String(e) };
      }
    })();
  });

  const rows = $derived(
    $envelopes.filter((e) => e.folderId === selectedFolder).sort((a, b) => b.dateMs - a.dateMs),
  );

  function selectFolder(id: string): void {
    selectedFolder = id;
    if (selectedId !== "@file") {
      selectedId = null;
      reading = null;
    }
    composing = false;
  }

  function selectMessage(id: string): void {
    selectedId = id;
    composing = false;
    markRead(id);
    void openMessage(id).then((m) => {
      if (selectedId === id) reading = m;
    });
  }

  function startCompose(to = "", subject = "", body = ""): void {
    preset = { to, subject, body };
    composing = true;
  }

  function composeDone(draftId: string | null): void {
    composing = false;
    if (draftId) {
      selectedFolder = "drafts";
      selectMessage(draftId);
    }
  }

  function reply(): void {
    if (!reading) return;
    startCompose(reading.from ?? "", reading.subject ? `Re: ${reading.subject}` : "");
  }
  function forward(): void {
    if (!reading) return;
    startCompose("", reading.subject ? `Fwd: ${reading.subject}` : "", reading.text ? `\n\n${reading.text}` : "");
  }
  function archiveSelected(): void {
    if (!selectedId || selectedId === "@file") return;
    moveMessage(selectedId, "archive");
    selectedId = null;
    reading = null;
  }
  function deleteSelected(): void {
    if (!selectedId || selectedId === "@file") return;
    if (selectedFolder === "trash") deleteForever(selectedId);
    else moveMessage(selectedId, "trash");
    selectedId = null;
    reading = null;
  }

  // Spelled out for the key gate, like the rail's names.
  const FOLDER_NAMES: Record<FolderKind, string> = {
    inbox: "ml.folder.inbox",
    sent: "ml.folder.sent",
    drafts: "ml.folder.drafts",
    archive: "ml.folder.archive",
    trash: "ml.folder.trash",
  };
  // The bar names the place: compose, the open message, or the folder.
  const barTitle = $derived.by(() => {
    if (composing) return $t("ml.compose.title");
    if (selectedId === "@file" && $openedFile) return $openedFile.subject ?? $t("ml.openedFile");
    if (selectedId && reading) return reading.subject ?? "-";
    const kind = $folders.find((f) => f.id === selectedFolder)?.kind;
    return kind ? $t(FOLDER_NAMES[kind]) : $t("ml.app.title");
  });

  const showActions = $derived(!composing && selectedId !== null && selectedId !== "@file" && reading !== null);

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }

  async function startDrag(e: PointerEvent) {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* standalone (vite) has no toplevel to drag */
    }
  }

  async function toggleMax(e: MouseEvent) {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      if (await w.isMaximized()) await w.unmaximize();
      else await w.maximize();
    } catch {
      /* no window in standalone */
    }
  }
</script>

<SidebarProvider class="h-screen min-h-0 overflow-hidden">
  <FolderRail activeFolder={composing ? null : selectedFolder} onselect={selectFolder} oncompose={() => startCompose()} />

  <SidebarInset class="h-svh min-h-0">
    <!-- The header is a drag surface (a non-keyboard pointer interaction); its
         actual controls are the accessible buttons inside it, so the
         static-interaction lint is a false positive here. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header
      onpointerdown={startDrag}
      ondblclick={toggleMax}
      class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background px-2"
    >
      <SidebarTrigger class="-ml-1" />
      <Separator orientation="vertical" class="me-1 h-4" />
      <span class="select-none truncate text-sm font-medium text-foreground">{barTitle}</span>
      <div class="flex-1"></div>
      {#if showActions}
        <IconAction label={$t("ml.reply")} size="control" onclick={reply}>
          <Reply size={15} strokeWidth={1.75} />
        </IconAction>
        <IconAction label={$t("ml.forward")} size="control" onclick={forward}>
          <Forward size={15} strokeWidth={1.75} />
        </IconAction>
        <IconAction label={$t("ml.archive")} size="control" onclick={archiveSelected}>
          <Archive size={15} strokeWidth={1.75} />
        </IconAction>
        <IconAction label={$t("ml.delete")} size="control" onclick={deleteSelected}>
          <Trash2 size={15} strokeWidth={1.75} />
        </IconAction>
      {/if}
      <WindowButtons />
    </header>

    <div class="body-row">
      {#if $folders.length > 0}
        <div class="list-col">
          {#if $mailboxMocked}
            <p class="sample">{$t("ml.sample")}</p>
          {/if}
          {#if $openedFile}
            <button
              type="button"
              class="opened-file"
              class:on={selectedId === "@file"}
              id="opened-file"
              onclick={() => {
                selectedId = "@file";
                composing = false;
              }}
            >
              <FileText size={14} strokeWidth={1.75} aria-hidden="true" />
              <span class="of-body">
                <span class="of-label">{$t("ml.openedFile")}</span>
                <span class="of-subject">{$openedFile.subject ?? "-"}</span>
              </span>
            </button>
          {/if}
          <MessageList {rows} {selectedId} onselect={selectMessage} />
        </div>
      {/if}

      <div class="pane">
        {#if composing}
          {#key preset}
            <ComposeView presetTo={preset.to} presetSubject={preset.subject} presetBody={preset.body} ondone={composeDone} />
          {/key}
        {:else if failure}
          <div class="center">
            <p class="note bad" role="alert">
              {#if failure.problem === "unreadable"}{$t("ml.failed.unreadable", { why: failure.why })}
              {:else if failure.problem === "not-a-message"}{$t("ml.failed.notAMessage")}
              {:else}{$t("ml.failed.other", { reason: failure.reason })}{/if}
            </p>
          </div>
        {:else if selectedId === "@file" && $openedFile}
          <MessageView message={$openedFile} />
        {:else if selectedId && reading}
          <MessageView message={reading} />
        {:else}
          <div class="center">
            <Mail size={28} strokeWidth={1.5} aria-hidden="true" />
            <p class="note">
              {#if $folders.length > 0}
                {$t("ml.noneSelected")}
              {:else if tauriAvailable}
                {$t("ml.unconnected")}
              {:else}
                {$t("ml.nothingOpen")}
              {/if}
            </p>
          </div>
        {/if}
      </div>
    </div>
  </SidebarInset>
</SidebarProvider>

<style>
  .body-row {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .list-col {
    display: flex;
    flex-direction: column;
    width: 21rem;
    min-height: 0;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--color-border-default, #2a2a2a);
  }
  .sample {
    margin: 0;
    padding: 0.4rem 0.7rem;
    border-bottom: 1px solid var(--color-border-default, #2a2a2a);
    font-size: var(--text-2xs, 11px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .opened-file {
    display: flex;
    gap: 0.5rem;
    align-items: center;
    margin: 0.35rem 0.35rem 0;
    padding: 0.45rem 0.55rem;
    border: 1px dashed var(--color-border-default, #2a2a2a);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .opened-file:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .opened-file.on {
    background: color-mix(in srgb, var(--color-fg-primary) 9%, transparent);
  }
  .opened-file :global(svg) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .of-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .of-label {
    font-size: var(--text-2xs, 11px);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .of-subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--text-sm, 13px);
  }
  .pane {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
  }
  .center {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    align-items: center;
    justify-content: center;
    flex: 1;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .note {
    margin: 0;
    max-width: 26rem;
    text-align: center;
    font-size: var(--text-sm, 13px);
    line-height: 1.5;
    color: var(--color-fg-secondary, #a3a3a3);
  }
  .note.bad {
    color: var(--color-warning, #eab308);
  }
</style>
