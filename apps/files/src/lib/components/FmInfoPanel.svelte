<script lang="ts">
  /// The on-demand info panel (KG quiet place #2). A drawer-style inspector: an
  /// identity block (icon, name, kind and size) on top, then prioritised
  /// sections (Where from, Related with the as-of view, Permissions, Photo
  /// details), each rendered only when it has something to show. Permissions are
  /// edited as plain per-role access, applied immediately; the octal is gone.
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { X, ChevronRight, ChevronDown } from "lucide-svelte";
  import {
    entryIcon,
    formatModified,
    formatSize,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { openPath } from "$lib/adapter";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Button } from "@arlen/ui-kit/components/ui/button";

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
      created_unix?: number | null;
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
  const isJpeg = $derived(/\.jpe?g$/i.test(name));

  const kindLabel = (kind: string): string =>
    kind === "directory" ? "Folder" : kind === "symlink" ? "Link" : "File";

  // ---- As-of time-travel for the Related lineage --------------------------
  // Re-read project membership at a past time via `files_verwandt_as_of`. Off by
  // default ("Now"); the presets are relative to the current moment. Only
  // project membership is bitemporal, so this is the meaningful slice.
  const AS_OF_OPTIONS = [
    { value: "now", label: "Now" },
    { value: "1d", label: "1 day ago" },
    { value: "1w", label: "1 week ago" },
    { value: "1m", label: "1 month ago" },
    { value: "3m", label: "3 months ago" },
  ];
  const DAY_MICROS = 86_400_000_000;
  const AS_OF_DELTAS: Record<string, number> = {
    "1d": DAY_MICROS,
    "1w": 7 * DAY_MICROS,
    "1m": 30 * DAY_MICROS,
    "3m": 90 * DAY_MICROS,
  };

  let asOfChoice = $state("now");
  let asOfMicros = $state<number | null>(null);
  const asOfVerwandt = writable<Info["verwandt"]>([]);

  // Reset transient view state when the inspected file changes.
  let advancedOpen = $state(false);
  let photoOpen = $state(false);
  $effect(() => {
    path;
    asOfChoice = "now";
    asOfMicros = null;
    advancedOpen = false;
    photoOpen = false;
  });

  function setAsOf(v: string) {
    asOfChoice = v;
    asOfMicros = v === "now" ? null : Date.now() * 1000 - AS_OF_DELTAS[v];
  }

  $effect(() => {
    const p = path;
    const t = asOfMicros;
    if (t === null) {
      asOfVerwandt.set([]);
      return;
    }
    invoke<Info["verwandt"]>("files_verwandt_as_of", { path: p, asOfMicros: t })
      .then((r) => asOfVerwandt.set(r))
      .catch(() => asOfVerwandt.set([]));
  });

  const asOfLabel = $derived(
    AS_OF_OPTIONS.find((o) => o.value === asOfChoice)?.label ?? "Now",
  );

  // ---- Permissions, as plain per-role access ------------------------------
  // The mode's permission bits, decoded into a Read & write / Read only / No
  // access choice per role. Changes apply immediately (no octal, no Save): we
  // reassemble the mode, write it, and re-read so the panel shows what landed.
  const PERM_OPTIONS = [
    { value: "rw", label: "Read & write" },
    { value: "r", label: "Read only" },
    { value: "none", label: "No access" },
  ];
  const permMode = $derived(($info?.conventional.mode ?? 0) & 0o777);
  const roleAccess = (bits: number): string =>
    bits & 4 ? (bits & 2 ? "rw" : "r") : "none";
  const accessBits = (a: string): number => (a === "rw" ? 6 : a === "r" ? 4 : 0);

  const ownerAccess = $derived(roleAccess((permMode >> 6) & 7));
  const groupAccess = $derived(roleAccess((permMode >> 3) & 7));
  const othersAccess = $derived(roleAccess(permMode & 7));
  const runnable = $derived((permMode & 0o111) !== 0);

  let permSaving = $state(false);
  let permError = $state(false);

  async function writeMode(mode: number) {
    permSaving = true;
    permError = false;
    try {
      await invoke("files_set_permissions", { path, mode });
      const i = await invoke<Info>("files_info", { path });
      info.set(i);
    } catch {
      permError = true;
    }
    permSaving = false;
  }

  function setRole(role: "owner" | "group" | "others", a: string) {
    const m = permMode;
    const parts = { owner: (m >> 6) & 7, group: (m >> 3) & 7, others: m & 7 };
    parts[role] = accessBits(a) | (parts[role] & 1); // preserve the execute bit
    void writeMode((parts.owner << 6) | (parts.group << 3) | parts.others);
  }

  function setRunnable(on: boolean) {
    void writeMode(on ? permMode | 0o111 : permMode & ~0o111);
  }

  // ---- EXIF (the media half of editable metadata, JPEG only) --------------
  interface ExifEdits {
    description: string | null;
    artist: string | null;
    copyright: string | null;
  }
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

  const created = $derived(
    $info?.conventional.created_unix
      ? formatModified($info.conventional.created_unix)
      : null,
  );
</script>

<aside class="panel" aria-label="Info">
  <header class="ident">
    <div class="ident-icon">
      {#if Icon}<Icon size={26} strokeWidth={1.25} />{/if}
    </div>
    <div class="ident-text">
      <span class="ident-name" title={name}>{name}</span>
      {#if $info}
        <span class="ident-sub">
          {kindLabel($info.conventional.kind)}{$info.conventional.kind !==
          "directory"
            ? ` · ${formatSize($info.conventional.size)}`
            : ""}
        </span>
      {/if}
    </div>
    <button class="close" aria-label="Close info" onclick={() => onclose?.()}>
      <X size={14} strokeWidth={2} />
    </button>
  </header>

  {#if $info}
    <div class="facts">
      <div class="kv">
        <span class="kv-label">Modified</span>
        <span class="kv-value">{formatModified($info.conventional.modified_unix)}</span>
      </div>
      {#if created}
        <div class="kv">
          <span class="kv-label">Created</span>
          <span class="kv-value">{created}</span>
        </div>
      {/if}
    </div>

    {#if $info.woher.length > 0}
      <section class="sec">
        <span class="sec-title">Where from</span>
        {#each $info.woher as line (line.label + line.detail)}
          <div class="prov">
            <span class="prov-label">{line.label}</span>
            <span class="prov-value">{line.detail}</span>
          </div>
        {/each}
      </section>
    {/if}

    {#if $info.verwandt.length > 0}
      {@const rels = asOfMicros === null ? $info.verwandt : $asOfVerwandt}
      <section class="sec">
        <div class="sec-head">
          <span class="sec-title">Related</span>
          <div class="asof">
            <span class="asof-key">As of</span>
            <PopoverSelect
              value={asOfChoice}
              options={AS_OF_OPTIONS}
              width="8rem"
              ariaLabel="View related projects as of a past time"
              onchange={setAsOf}
            />
          </div>
        </div>
        {#if asOfMicros !== null}
          <span class="note">Past view, as of {asOfLabel.toLowerCase()}</span>
        {/if}
        {#each rels as line (line.label + line.target_id)}
          <button
            type="button"
            class="rel"
            onclick={() => onnavigate?.(`project:${line.target_id}`)}
          >
            <span class="rel-label">{line.label}</span>
            <span class="rel-target">{line.target}</span>
            <ChevronRight class="rel-chev" size={14} strokeWidth={2} />
          </button>
        {/each}
        {#if asOfMicros !== null && rels.length === 0}
          <span class="empty">No related projects at that time.</span>
        {/if}
      </section>
    {/if}

    {#if $info.conventional.kind !== "symlink"}
      <section class="sec">
        <span class="sec-title">Permissions</span>
        <div class="perm">
          <span class="perm-label">You</span>
          <div class="perm-ctl">
            <PopoverSelect
              value={ownerAccess}
              options={PERM_OPTIONS}
              width="100%"
              ariaLabel="Your access"
              disabled={permSaving}
              onchange={(v) => setRole("owner", v)}
            />
          </div>
        </div>
        <div class="perm">
          <span class="perm-label">Others</span>
          <div class="perm-ctl">
            <PopoverSelect
              value={othersAccess}
              options={PERM_OPTIONS}
              width="100%"
              ariaLabel="Everyone else's access"
              disabled={permSaving}
              onchange={(v) => setRole("others", v)}
            />
          </div>
        </div>

        <button class="disc" onclick={() => (advancedOpen = !advancedOpen)}>
          <ChevronDown class="disc-chev" size={13} strokeWidth={2} data-open={advancedOpen} />
          Advanced
        </button>
        {#if advancedOpen}
          <div class="perm">
            <span class="perm-label">Group</span>
            <div class="perm-ctl">
              <PopoverSelect
                value={groupAccess}
                options={PERM_OPTIONS}
                width="100%"
                ariaLabel="Group access"
                disabled={permSaving}
                onchange={(v) => setRole("group", v)}
              />
            </div>
          </div>
          {#if $info.conventional.kind === "file"}
            <div class="perm perm-toggle">
              <span class="perm-label-wide">Allow running as a program</span>
              <Switch
                value={runnable}
                disabled={permSaving}
                ariaLabel="Allow running as a program"
                onchange={setRunnable}
              />
            </div>
          {/if}
        {/if}
        {#if permError}
          <span class="err">Couldn't change permissions.</span>
        {/if}
      </section>
    {/if}

    {#if isJpeg && $info.conventional.kind === "file" && exifLoaded}
      <section class="sec">
        <button class="disc disc-title" onclick={() => (photoOpen = !photoOpen)}>
          <ChevronDown class="disc-chev" size={13} strokeWidth={2} data-open={photoOpen} />
          Photo details
        </button>
        {#if photoOpen}
          <label class="field">
            <span class="field-label">Description</span>
            <Input bind:value={exifDraft.description} aria-invalid={exifError} />
          </label>
          <label class="field">
            <span class="field-label">Artist</span>
            <Input bind:value={exifDraft.artist} aria-invalid={exifError} />
          </label>
          <label class="field">
            <span class="field-label">Copyright</span>
            <Input bind:value={exifDraft.copyright} aria-invalid={exifError} />
          </label>
          <Button
            variant="outline"
            size="sm"
            class="field-save"
            disabled={exifSaving}
            onclick={() => void saveExif()}
          >
            Save
          </Button>
        {/if}
      </section>
    {/if}

    {#if $info.zugriff.readable_by.length > 0}
      <section class="sec">
        <span class="sec-title">Access</span>
        <div class="kv">
          <span class="kv-label">Readable by</span>
          <span class="kv-value">{$info.zugriff.readable_by.join(", ")}</span>
        </div>
        <Button
          variant="ghost"
          size="sm"
          class="field-save"
          onclick={() => void openPath($info.zugriff.manage_link)}
        >
          Manage access in Settings
        </Button>
      </section>
    {/if}
  {/if}
</aside>

<style>
  .panel {
    width: 17rem;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 1rem;
    padding: 1rem;
    border-left: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    overflow-y: auto;
  }

  /* Identity block: icon, name + a kind/size subline, close. */
  .ident {
    display: flex;
    align-items: flex-start;
    gap: 0.625rem;
  }
  .ident-icon {
    flex-shrink: 0;
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    margin-top: 0.0625rem;
  }
  .ident-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }
  .ident-name {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ident-sub {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .close {
    flex-shrink: 0;
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
  .close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  /* Read-only key/value, used by facts + where-from + access. A consistent
     narrow label column keeps every row aligned; the value flexes + wraps. */
  .facts {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .kv {
    display: flex;
    gap: 0.75rem;
    font-size: 0.75rem;
    line-height: 1.45;
  }
  .kv-label {
    flex: 0 0 4.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .kv-value {
    flex: 1;
    min-width: 0;
    color: var(--foreground);
    overflow-wrap: anywhere;
  }

  /* Where-from entries: a muted label over its value. Stacked (not a column)
     so it stays aligned whatever the label length ("Also accessed by"). */
  .prov {
    display: flex;
    flex-direction: column;
    gap: 0.0625rem;
    font-size: 0.75rem;
    line-height: 1.4;
  }
  .prov-label {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .prov-value {
    color: var(--foreground);
    overflow-wrap: anywhere;
  }

  /* A labelled section, divided from the one above by a hairline. */
  .sec {
    display: flex;
    flex-direction: column;
    gap: 0.4375rem;
    padding-top: 0.875rem;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .sec-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }
  .sec-title {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  /* As-of control in the Related header. */
  .asof {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    flex-shrink: 0;
  }
  .asof-key {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    white-space: nowrap;
  }
  .note {
    font-size: 0.6875rem;
    color: var(--color-warning, #d4b483);
  }
  .empty {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  /* A clickable Related row: a quiet hoverable row, not a web link. */
  .rel {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    width: calc(100% + 0.75rem);
    margin: 0 -0.375rem;
    padding: 0.3125rem 0.375rem;
    border: none;
    background: transparent;
    border-radius: var(--radius-chip);
    font-size: 0.75rem;
    text-align: left;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .rel:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .rel-label {
    flex: 0 0 auto;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .rel-target {
    flex: 1;
    min-width: 0;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.rel-chev) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  /* Permissions: a label + a per-role access select, on one line. */
  .perm {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .perm-label {
    flex: 0 0 3.25rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .perm-ctl {
    flex: 1;
    min-width: 0;
  }
  .perm-toggle {
    justify-content: space-between;
  }
  .perm-label-wide {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }

  /* A lightweight disclosure (Advanced, Photo details). */
  .disc {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    align-self: flex-start;
    padding: 0.125rem 0;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.75rem;
  }
  .disc:hover {
    color: var(--foreground);
  }
  .disc-title {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
  :global(.disc-chev) {
    transition: transform var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  :global(.disc-chev[data-open="true"]) {
    transform: rotate(180deg);
  }

  /* EXIF edit fields: a stacked label over a kit Input. */
  .field {
    display: flex;
    flex-direction: column;
    gap: 0.1875rem;
  }
  .field-label {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  :global(.field-save) {
    align-self: flex-start;
    margin-top: 0.125rem;
  }

  .err {
    font-size: 0.6875rem;
    color: var(--color-error, #e5484d);
  }
</style>
