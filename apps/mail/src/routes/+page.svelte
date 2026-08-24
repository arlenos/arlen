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
  import { displayName, threadKey } from "$lib/wording";
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
  import ThreadView from "$lib/components/ThreadView.svelte";
  import ComposeView from "$lib/components/ComposeView.svelte";
  import {
    folders,
    envelopes,
    type Envelope,
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
  /// The selected message ids. One id reads; several arm the bulk actions.
  /// "@file" is the launched-file pseudo-selection and never joins a set.
  let selected = $state<Set<string>>(new Set());
  let fileOpen = $state(false);
  /// The open conversation: its subject and its messages in sent order. One
  /// message is the common case and renders as the plain reading surface.
  let reading = $state<{ subject: string; messages: Message[] } | null>(null);
  /// A row that was clicked and did not open. Only ever true under a host: with
  /// no host there is nothing to have refused, and the sample stands in.
  let readFailed = $state(false);
  let composing = $state(false);
  let preset = $state<{ to: string; subject: string; body: string }>({ to: "", subject: "", body: "" });

  type Failure =
    | { problem: "launch"; reason: string }
    | { problem: "unreadable"; why: string }
    | { problem: "not-a-message" }
    | { problem: "other"; reason: string };
  let failure = $state<Failure | null>(null);

  onMount(() => {
    void loadMailbox();
    if (!tauriAvailable) return;
    void (async () => {
      // A THROW AND A NULL ARE DIFFERENT ANSWERS. `null` means the window was
      // opened with no file, which is how the mailbox is normally started; a
      // throw means the host could not say what it was asked to open, and folding
      // the two together tells somebody who just double-clicked a message that no
      // account is connected. The reader fixed the same shape one app over.
      let launched: string | null = null;
      try {
        launched = await invoke<string | null>("launch_file");
      } catch (e) {
        failure = { problem: "launch", reason: String(e) };
        return;
      }
      if (!launched) return;
      try {
        const m = await invoke<Message>("mail_read", { path: launched });
        openedFile.set(m);
        fileOpen = true;
        selected = new Set();
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

  /// The reading folders thread by subject ACROSS inbox/sent/archive, so your
  /// own reply sits in the conversation it answers. Drafts and trash list
  /// flat: a draft is not a conversation event, and a bin is a bin.
  const THREAD_SCOPE = ["inbox", "sent", "archive"];
  const threaded = $derived(THREAD_SCOPE.includes(selectedFolder));

  const grouped = $derived.by(() => {
    if (!threaded) {
      const flat = $envelopes
        .filter((e) => e.folderId === selectedFolder)
        .sort((a, b) => b.dateMs - a.dateMs);
      return {
        rows: flat as (Envelope & { count?: number })[],
        members: new Map(flat.map((e) => [e.id, [e]])),
      };
    }
    const buckets = new Map<string, Envelope[]>();
    for (const e of $envelopes) {
      if (!THREAD_SCOPE.includes(e.folderId)) continue;
      const key = threadKey(e.subject);
      const list = buckets.get(key);
      if (list) list.push(e);
      else buckets.set(key, [e]);
    }
    const rows: (Envelope & { count?: number })[] = [];
    const members = new Map<string, Envelope[]>();
    for (const [key, list] of buckets) {
      if (!list.some((e) => e.folderId === selectedFolder)) continue;
      const asc = [...list].sort((a, b) => a.dateMs - b.dateMs);
      const newest = asc[asc.length - 1];
      const names = [...new Set(asc.map((e) => displayName(e.from)))];
      const id = `t:${key}`;
      members.set(id, asc);
      rows.push({
        id,
        folderId: selectedFolder,
        from: names.length > 2 ? `${names[0]}, ${names[1]} +${names.length - 2}` : names.join(", "),
        // The FIRST message's subject names the conversation - "Re: Re: X"
        // is a wire artefact, not a title.
        subject: asc[0].subject,
        snippet: newest.snippet,
        dateMs: newest.dateMs,
        unread: asc.some((e) => e.unread && e.folderId === selectedFolder),
        count: asc.length > 1 ? asc.length : undefined,
      });
    }
    rows.sort((a, b) => b.dateMs - a.dateMs);
    return { rows, members };
  });
  const rows = $derived(grouped.rows);

  function selectFolder(id: string): void {
    selectedFolder = id;
    if (!fileOpen) {
      selected = new Set();
      reading = null;
    }
    composing = false;
  }

  /// Open one row: the conversation's messages load in sent order and reading
  /// them marks them read.
  function loadRow(id: string): void {
    const mem = grouped.members.get(id) ?? [];
    for (const e of mem) markRead(e.id);
    void Promise.all(mem.map((e) => openMessage(e.id))).then((list) => {
      if (!(selected.size === 1 && selected.has(id))) return;
      const messages = list.filter((m): m is Message => m !== null);
      if (messages.length === 0) {
        // Nothing came back. This used to return in silence, which read as "the
        // row you clicked has no message in it" - and before that it showed a
        // fixture, which read as somebody else's mail.
        readFailed = tauriAvailable;
        reading = null;
        return;
      }
      readFailed = false;
      reading = { subject: mem[0].subject, messages };
    });
  }

  /// A plain click or a keyboard step: single-select and read.
  function openOne(id: string): void {
    selected = new Set([id]);
    fileOpen = false;
    composing = false;
    loadRow(id);
  }

  /// Ctrl/shift selection from the list: several rows arm the bulk actions,
  /// and the reading pane steps back to the count.
  function selectionChanged(sel: Set<string>): void {
    selected = sel;
    fileOpen = false;
    composing = false;
    if (sel.size !== 1) reading = null;
    else loadRow([...sel][0]);
  }

  function startCompose(to = "", subject = "", body = ""): void {
    preset = { to, subject, body };
    composing = true;
  }

  function composeDone(draftId: string | null): void {
    composing = false;
    if (draftId) {
      selectedFolder = "drafts";
      openOne(draftId);
    }
  }

  /// Reply and forward speak to the NEWEST message of the conversation.
  function reply(): void {
    const m = reading?.messages[reading.messages.length - 1];
    if (!m) return;
    startCompose(m.from ?? "", m.subject ? `Re: ${m.subject}` : "");
  }
  function forward(): void {
    const m = reading?.messages[reading.messages.length - 1];
    if (!m) return;
    startCompose("", m.subject ? `Fwd: ${m.subject}` : "", m.text ? `\n\n${m.text}` : "");
  }
  /// Bulk moves act on the CURRENT folder's members of each conversation -
  /// deleting a thread from the inbox does not eat your sent copy.
  function folderMembers(): string[] {
    const out: string[] = [];
    for (const id of selected)
      for (const e of grouped.members.get(id) ?? []) if (e.folderId === selectedFolder) out.push(e.id);
    return out;
  }
  function archiveSelected(): void {
    for (const id of folderMembers()) moveMessage(id, "archive");
    if (selected.size > 0) {
      selected = new Set();
      reading = null;
    }
  }
  function deleteSelected(): void {
    for (const id of folderMembers()) {
      if (selectedFolder === "trash") deleteForever(id);
      else moveMessage(id, "trash");
    }
    if (selected.size > 0) {
      selected = new Set();
      reading = null;
    }
  }

  // Spelled out for the key gate, like the rail's names.
  const FOLDER_NAMES: Record<FolderKind, string> = {
    inbox: "ml.folder.inbox",
    sent: "ml.folder.sent",
    drafts: "ml.folder.drafts",
    archive: "ml.folder.archive",
    trash: "ml.folder.trash",
  };
  // The bar names the place: compose, the open message, a selection count, or
  // the folder.
  const barTitle = $derived.by(() => {
    if (composing) return $t("ml.compose.title");
    if (fileOpen && $openedFile) return $openedFile.subject ?? $t("ml.openedFile");
    if (selected.size > 1) return $t("ml.selectedCount", { n: selected.size });
    if (selected.size === 1 && reading) return reading.subject;
    const kind = $folders.find((f) => f.id === selectedFolder)?.kind;
    return kind ? $t(FOLDER_NAMES[kind]) : $t("ml.app.title");
  });

  /// One message read: everything. Several: only the actions that make sense
  /// for a pile (archive, delete) - a bulk Reply would be a lie.
  const showSingleActions = $derived(!composing && !fileOpen && selected.size === 1 && reading !== null);
  const showBulkActions = $derived(!composing && !fileOpen && selected.size > 1);

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
      {#if showSingleActions}
        <IconAction label={$t("ml.reply")} size="control" onclick={reply}>
          <Reply size={15} strokeWidth={1.75} />
        </IconAction>
        <IconAction label={$t("ml.forward")} size="control" onclick={forward}>
          <Forward size={15} strokeWidth={1.75} />
        </IconAction>
      {/if}
      {#if showSingleActions || showBulkActions}
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
              class:on={fileOpen}
              id="opened-file"
              onclick={() => {
                fileOpen = true;
                selected = new Set();
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
          <MessageList
            {rows}
            {selected}
            onchange={selectionChanged}
            onopen={openOne}
            onarchive={archiveSelected}
            ondelete={deleteSelected}
          />
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
              {#if failure.problem === "launch"}{$t("ml.failed.launch", { reason: failure.reason })}
              {:else if failure.problem === "unreadable"}{$t("ml.failed.unreadable", { why: failure.why })}
              {:else if failure.problem === "not-a-message"}{$t("ml.failed.notAMessage")}
              {:else}{$t("ml.failed.other", { reason: failure.reason })}{/if}
            </p>
          </div>
        {:else if fileOpen && $openedFile}
          <MessageView message={$openedFile} />
        {:else if readFailed}
          <div class="center">
            <Mail size={28} strokeWidth={1.5} aria-hidden="true" />
            <p class="note" role="alert">{$t("ml.openFailed")}</p>
          </div>
        {:else if selected.size === 1 && reading}
          {#if reading.messages.length === 1}
            <MessageView message={reading.messages[0]} />
          {:else}
            <ThreadView subject={reading.subject} messages={reading.messages} />
          {/if}
        {:else if selected.size > 1}
          <div class="center">
            <Mail size={28} strokeWidth={1.5} aria-hidden="true" />
            <p class="note">{$t("ml.selectedCount", { n: selected.size })}</p>
          </div>
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
