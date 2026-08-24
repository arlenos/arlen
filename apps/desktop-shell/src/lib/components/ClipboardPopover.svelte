<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The clipboard-history panel (clipboard-api.md): the last thirty texts,
  /// searchable, one click to copy back. Selection copies and closes; delete
  /// is per row; Clear all confirms in place because the backend deliberately
  /// does not. The empty panel names the design behind it - history lives
  /// only while the shell runs - so a fresh login does not read as data loss.
  /// Opened by keybinding (the compositor seam); `?popover=clipboard` under
  /// vite until that lands.
  import { Clipboard, X } from "lucide-svelte";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import {
    Command,
    CommandInput,
    CommandList,
    CommandItem,
    CommandEmpty,
  } from "@arlen/ui-kit/components/ui/command";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import {
    clipEntries,
    clipEnabled,
    clipMocked,
    clipUnavailable,
    clipCopyFailed,
    loadClipboardPanel,
    copyPanelEntry,
    deletePanelEntry,
    clearPanel,
    type ClipboardPanelEntry,
  } from "$lib/stores/clipboardPanel";

  let query = $state("");
  let confirmClear = $state(false);
  let confirmTimer: ReturnType<typeof setTimeout> | undefined;

  const open = $derived($activePopover === "clipboard");

  // Fresh snapshot per open, stale filter never shown; while open, a change
  // to the ring buffer (a new copy, an SDK delete) refreshes the list.
  $effect(() => {
    if (!open) return;
    query = "";
    confirmClear = false;
    void loadClipboardPanel();
    let unlisten: UnlistenFn | undefined;
    void listen("arlen://clipboard-changed", () => void loadClipboardPanel())
      .then((u) => (unlisten = u))
      .catch(() => {
        // No event bridge under vite; the fixture does not change anyway.
      });
    return () => unlisten?.();
  });

  async function pick(entry: ClipboardPanelEntry) {
    if (await copyPanelEntry(entry.id)) closePopover();
  }

  function onClear() {
    if (!confirmClear) {
      confirmClear = true;
      clearTimeout(confirmTimer);
      confirmTimer = setTimeout(() => (confirmClear = false), 3000);
      return;
    }
    clearTimeout(confirmTimer);
    confirmClear = false;
    void clearPanel();
  }

  // Compact ages, same register as the undo panel ("now", "4m", "2h").
  function ago(ms: number): string {
    const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
    if (s < 90) return "now";
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m`;
    return `${Math.floor(m / 60)}h`;
  }

  // The short app handle from the id ("org.arlen.files" -> "files"); an entry
  // captured with no focused window has none and shows none.
  function appOf(e: ClipboardPanelEntry): string {
    const seg = e.sourceAppId.split(".").filter(Boolean);
    return seg.length ? seg[seg.length - 1] : "";
  }

  function firstLine(s: string): string {
    const i = s.indexOf("\n");
    return i === -1 ? s : s.slice(0, i);
  }

  function extraLines(s: string): number {
    return s.split("\n").length - 1;
  }
</script>

<ShellPopover id="clipboard" width={400} right={16} bodyPadding="0px" bodyGap="0px">
  {#snippet header()}
    <PopoverHeader icon={Clipboard} title={$t("sh.clip.title")} />
  {/snippet}

  {#if $clipEnabled === false}
    <p class="clip-said">{$t("sh.clip.off")}</p>
  {:else if $clipUnavailable}
    <p class="clip-said">{$t("sh.clip.unavailable")}</p>
  {:else if $clipEntries && $clipEntries.length === 0}
    <!-- Empty is the designed state after every login (FA12: nothing
         persists), so the sentence carries the why. -->
    <p class="clip-said">{$t("sh.clip.empty")}</p>
  {:else if $clipEntries}
    {#if $clipMocked}
      <p class="clip-sample">{$t("sh.clip.mocked")}</p>
    {/if}
    {#if $clipCopyFailed}
      <p class="clip-sample" role="alert">{$t("sh.clip.copyFailed")}</p>
    {/if}
    <Command>
      <CommandInput placeholder={$t("sh.clip.search")} autofocus bind:value={query} />
      <CommandList class="clip-list">
        <CommandEmpty>{$t("sh.clip.noMatches")}</CommandEmpty>
        {#each $clipEntries as e (e.id)}
          <CommandItem value={`${e.content} ${appOf(e)}`} onSelect={() => void pick(e)}>
            <span class="clip-row">
              <span class="clip-text">
                <span class="clip-snippet">{firstLine(e.content)}</span>
                <span class="clip-meta">
                  {#if extraLines(e.content) > 0}
                    <span>{$t("sh.clip.moreLines", { n: extraLines(e.content) })}</span>
                  {/if}
                  {#if appOf(e)}
                    <span>{appOf(e)}</span>
                  {/if}
                  <span>{ago(e.timestampMs)}</span>
                </span>
              </span>
              <button
                type="button"
                class="clip-delete"
                aria-label={$t("sh.clip.deleteAria")}
                onclick={(ev) => {
                  ev.stopPropagation();
                  void deletePanelEntry(e.id);
                }}
              >
                <X size={13} strokeWidth={2} />
              </button>
            </span>
          </CommandItem>
        {/each}
      </CommandList>
    </Command>
    <div class="clip-foot">
      <button type="button" class="clip-clear" class:arm={confirmClear} onclick={onClear}>
        {confirmClear ? $t("sh.clip.clearConfirm") : $t("sh.clip.clearAll")}
      </button>
    </div>
  {/if}
</ShellPopover>

<style>
  .clip-said {
    margin: 0;
    padding: 14px 12px;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
  }
  .clip-sample {
    margin: 0;
    padding: 8px 12px 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
  }

  :global(.clip-list) {
    /* Eight whole rows plus padding; the fold never lands mid-row. */
    max-height: 356px;
    padding: 4px;
    scrollbar-width: none;
  }

  .clip-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }
  .clip-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  /* The content in the content's own register: monospace, one line. */
  .clip-snippet {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
    color: var(--color-fg-shell);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .clip-meta {
    display: flex;
    gap: 8px;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-shell) 42%, transparent);
  }
  .clip-delete {
    display: none;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);
    cursor: pointer;
  }
  :global([data-selected]) .clip-delete,
  .clip-row:hover .clip-delete {
    display: inline-flex;
  }
  .clip-delete:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }

  .clip-foot {
    display: flex;
    justify-content: flex-end;
    padding: 6px 8px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
  .clip-clear {
    border: none;
    background: transparent;
    padding: 3px 8px;
    border-radius: var(--radius-chip, 4px);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
    cursor: pointer;
  }
  .clip-clear:hover {
    color: var(--color-fg-shell);
  }
  /* Armed: the second click destroys, so the button says so and warms up. */
  .clip-clear.arm {
    color: var(--color-warning, #ca8a04);
  }
</style>
