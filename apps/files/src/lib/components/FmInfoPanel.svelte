<script lang="ts">
  /// The on-demand info panel (KG quiet place #2): conventional
  /// Get-Info on top, then the graph sections — Where from, Related,
  /// Access — rendered only when the graph has something to say. The
  /// access view is read-only with one deep link; capabilities are
  /// managed in Settings, never here.
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { X, ChevronRight } from "lucide-svelte";
  import {
    entryIcon,
    formatModified,
    formatSize,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { openPath } from "$lib/adapter";

  let {
    path,
    entry,
    onclose,
    onnavigate,
  }: {
    /// The full path of the inspected entry.
    path: string;
    /// Its listing entry (for icon and name; null while unknown).
    entry: FileEntry | null;
    onclose?: () => void;
    /// Navigate to a KG location key (e.g. `project:<id>`); the parent wires
    /// this to the active controller, so a Related entry can open its lineage.
    onnavigate?: (location: string) => void;
  } = $props();

  interface Info {
    conventional: {
      kind: string;
      size: number;
      mode: number;
      modified_unix: number;
    };
    woher: { label: string; detail: string }[];
    verwandt: { label: string; target: string; target_id: string }[];
    zugriff: { readable_by: string[]; manage_link: string };
  }

  const info = writable<Info | null>(null);

  $effect(() => {
    const p = path;
    invoke<Info>("files_info", { path: p })
      .then((i) => info.set(i))
      .catch(() => info.set(null));
  });

  const name = $derived(path.split("/").filter(Boolean).pop() ?? "/");
  const Icon = $derived(entry ? entryIcon(entry) : null);

  const kindLabel = (kind: string): string =>
    kind === "directory" ? "Folder" : kind === "symlink" ? "Link" : "File";

  // The editable Unix permissions (chmod), the writable half of the metadata.
  // The draft is the octal the user edits; it prefills from the loaded mode and
  // saves through `files_set_permissions`, then re-reads the panel from disk.
  let modeDraft = $state("");
  let modeError = $state(false);
  let saving = $state(false);

  $effect(() => {
    const m = $info?.conventional.mode;
    modeDraft = m === undefined ? "" : (m & 0o777).toString(8).padStart(3, "0");
    modeError = false;
  });

  /// Render a mode's permission bits as the conventional `rwxr-xr-x` string.
  function rwx(mode: number): string {
    const part = (n: number) =>
      (n & 4 ? "r" : "-") + (n & 2 ? "w" : "-") + (n & 1 ? "x" : "-");
    return part((mode >> 6) & 7) + part((mode >> 3) & 7) + part(mode & 7);
  }

  async function saveMode() {
    if (!/^[0-7]{3,4}$/.test(modeDraft)) {
      modeError = true;
      return;
    }
    const mode = parseInt(modeDraft, 8);
    modeError = false;
    saving = true;
    try {
      await invoke("files_set_permissions", { path, mode });
      // Re-read so the displayed rwx + octal reflect what actually landed.
      const i = await invoke<Info>("files_info", { path });
      info.set(i);
    } catch {
      modeError = true;
    }
    saving = false;
  }

  // The writable EXIF tags (the media half of editable metadata), offered only
  // for JPEGs - the only format the backend write-back supports. The draft
  // prefills from `files_get_exif_tags` and saves through `files_set_exif_tags`,
  // which verifies the readback, then we re-read so the panel shows what landed.
  // A blank field saves as `null` (leave the tag unchanged), so this basic edit
  // never writes an empty tag; clearing a tag is a later refinement. The polished
  // unified panel is an arlen-ui pass; this is the coder's basic inline-edit.
  interface ExifEdits {
    description: string | null;
    artist: string | null;
    copyright: string | null;
  }

  const isJpeg = $derived(/\.jpe?g$/i.test(name));
  let exifDraft = $state({ description: "", artist: "", copyright: "" });
  let exifLoaded = $state(false);
  let exifError = $state(false);
  let exifSaving = $state(false);

  function fillExif(e: ExifEdits): void {
    exifDraft = {
      description: e.description ?? "",
      artist: e.artist ?? "",
      copyright: e.copyright ?? "",
    };
  }

  $effect(() => {
    const p = path;
    if (!isJpeg) {
      exifLoaded = false;
      return;
    }
    invoke<ExifEdits>("files_get_exif_tags", { path: p })
      .then((e) => {
        fillExif(e);
        exifError = false;
        exifLoaded = true;
      })
      .catch(() => {
        exifLoaded = false;
      });
  });

  async function saveExif() {
    const orNull = (s: string) => (s.trim().length > 0 ? s : null);
    exifSaving = true;
    exifError = false;
    try {
      await invoke("files_set_exif_tags", {
        path,
        description: orNull(exifDraft.description),
        artist: orNull(exifDraft.artist),
        copyright: orNull(exifDraft.copyright),
      });
      fillExif(await invoke<ExifEdits>("files_get_exif_tags", { path }));
    } catch {
      exifError = true;
    }
    exifSaving = false;
  }
</script>

<aside class="info-panel" aria-label="Info">
  <div class="ip-head">
    <span class="ip-name">{name}</span>
    <button class="ip-close" aria-label="Close info" onclick={() => onclose?.()}>
      <X size={14} strokeWidth={2} />
    </button>
  </div>

  <div class="ip-preview">
    {#if Icon}
      <Icon size={48} strokeWidth={1} />
    {/if}
  </div>

  {#if $info}
    <div class="ip-facts">
      <span>{kindLabel($info.conventional.kind)}</span>
      {#if $info.conventional.kind !== "directory"}
        <span>{formatSize($info.conventional.size)}</span>
      {/if}
      <span>changed {formatModified($info.conventional.modified_unix)}</span>
    </div>

    {#if $info.conventional.kind !== "symlink"}
      <div class="ip-section">
        <span class="ip-label">Permissions</span>
        <div class="ip-row">
          <span class="ip-key">Mode</span>
          <span class="ip-value">{rwx($info.conventional.mode)}</span>
        </div>
        <div class="ip-edit">
          <input
            class="ip-mode-input"
            class:ip-mode-error={modeError}
            bind:value={modeDraft}
            aria-label="Octal permissions"
            spellcheck="false"
            autocapitalize="off"
            autocomplete="off"
            maxlength="4"
          />
          <button
            class="ip-manage ip-save"
            disabled={saving}
            onclick={() => void saveMode()}
          >
            Save
          </button>
        </div>
      </div>
    {/if}

    {#if isJpeg && $info.conventional.kind === "file" && exifLoaded}
      <div class="ip-section">
        <span class="ip-label">Photo info</span>
        <label class="ip-field">
          <span class="ip-key">Description</span>
          <input
            class="ip-text-input"
            class:ip-mode-error={exifError}
            bind:value={exifDraft.description}
            spellcheck="false"
          />
        </label>
        <label class="ip-field">
          <span class="ip-key">Artist</span>
          <input
            class="ip-text-input"
            class:ip-mode-error={exifError}
            bind:value={exifDraft.artist}
            spellcheck="false"
          />
        </label>
        <label class="ip-field">
          <span class="ip-key">Copyright</span>
          <input
            class="ip-text-input"
            class:ip-mode-error={exifError}
            bind:value={exifDraft.copyright}
            spellcheck="false"
          />
        </label>
        <button
          class="ip-manage ip-save"
          disabled={exifSaving}
          onclick={() => void saveExif()}
        >
          Save
        </button>
      </div>
    {/if}

    {#if $info.woher.length > 0}
      <div class="ip-section">
        <span class="ip-label">Where from</span>
        {#each $info.woher as line (line.label + line.detail)}
          <div class="ip-row">
            <span class="ip-key">{line.label}</span>
            <span class="ip-value">{line.detail}</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if $info.verwandt.length > 0}
      <div class="ip-section">
        <span class="ip-label">Related</span>
        {#each $info.verwandt as line (line.label + line.target_id)}
          <button
            type="button"
            class="ip-rel"
            onclick={() => onnavigate?.(`project:${line.target_id}`)}
          >
            <span class="ip-key">{line.label}</span>
            <span class="ip-value">{line.target}</span>
            <ChevronRight class="ip-rel-chevron" size={14} strokeWidth={2} />
          </button>
        {/each}
      </div>
    {/if}

    {#if $info.zugriff.readable_by.length > 0}
      <div class="ip-section">
        <span class="ip-label">Access</span>
        <div class="ip-row">
          <span class="ip-key">Readable by</span>
          <span class="ip-value">{$info.zugriff.readable_by.join(", ")}</span>
        </div>
        <button
          class="ip-manage"
          onclick={() => void openPath($info.zugriff.manage_link)}
        >
          Manage access in Settings
        </button>
      </div>
    {/if}
  {/if}
</aside>

<style>
  .info-panel {
    width: 17rem;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    border-left: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    overflow-y: auto;
  }

  .ip-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ip-name {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ip-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ip-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  .ip-preview {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 7rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }

  .ip-facts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .ip-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ip-label {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ip-row {
    display: flex;
    gap: 8px;
    font-size: 0.75rem;
  }
  .ip-key {
    width: 6.5rem;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ip-value {
    flex: 1;
    min-width: 0;
    color: var(--foreground);
    overflow-wrap: anywhere;
  }

  /* A Related entry: a row that navigates to the linked KG node (its project).
     Reads as a quiet hoverable row, not a web link; the chevron signals it
     opens. */
  .ip-rel {
    display: flex;
    align-items: center;
    gap: 8px;
    width: calc(100% + 12px);
    margin: 0 -6px;
    padding: 4px 6px;
    border: none;
    background: transparent;
    border-radius: var(--radius-chip);
    font-size: 0.75rem;
    text-align: left;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .ip-rel:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .ip-rel .ip-value {
    color: var(--foreground);
  }
  :global(.ip-rel-chevron) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  .ip-manage {
    align-self: flex-start;
    margin-top: 4px;
    height: var(--height-control, 28px);
    padding: 0 12px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .ip-manage:hover {
    background: var(--control-bg-hover);
  }
  .ip-manage:disabled {
    opacity: 0.6;
  }

  .ip-edit {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
  }
  .ip-mode-input {
    width: 4rem;
    height: var(--height-control, 28px);
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
  }
  .ip-mode-error {
    border-color: var(--color-error, #e5484d);
  }
  .ip-save {
    margin-top: 0;
  }

  /* The EXIF edit rows: a stacked label + full-width text input, the column
     register the panel's sections already use. */
  .ip-field {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ip-text-input {
    width: 100%;
    height: var(--height-control, 28px);
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.75rem;
  }
</style>
