<script lang="ts">
  /// Headless render harness for FmInfoPanel. UI-AFFORDANCE verification ONLY,
  /// NOT a behaviour claim. Mocks the daemon over Tauri IPC (only when no Tauri
  /// runtime is present, so it can never hijack the real app) and renders the
  /// real FmInfoPanel across its states: a file with related projects + the
  /// as-of view, an image with EXIF + permissions, a folder, a symlink. The real
  /// permission writes + KG behaviour are proven by the coder's seed + tests, not
  /// this mock. Not shipped in any nav; a dev/test route only.
  import { onMount } from "svelte";
  import { tauriAvailable } from "$lib/tauri";
  import FmInfoPanel from "$lib/components/FmInfoPanel.svelte";
  import FmStatusBar from "$lib/components/FmStatusBar.svelte";
  import OpsOverlays from "$lib/components/OpsOverlays.svelte";
  import { opError } from "$lib/stores/ops";
  import ProvenanceHalo from "$lib/components/ProvenanceHalo.svelte";
  import type { FileEntry } from "@arlen/ui-kit/components/browser";

  const FILE = "/demo/thesis-draft.md";
  const IMAGE = "/demo/inn-sunset.jpg";
  const FOLDER = "/demo/Projects";
  const SYMLINK = "/demo/shortcut";
  const REALFILE = "/demo/thesis-live.md"; // provenance_of IS mocked for this one

  // A stand-in thumbnail data URI (the real `files_thumbnail` returns one).
  const thumbSvg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#e8a06a"/><stop offset="0.6" stop-color="#9c5a6a"/><stop offset="1" stop-color="#2a2f4a"/></linearGradient></defs><rect width="320" height="200" fill="url(#g)"/><circle cx="160" cy="118" r="32" fill="#f4d28a"/></svg>';
  const thumbDataUri =
    typeof btoa !== "undefined" ? `data:image/svg+xml;base64,${btoa(thumbSvg)}` : null;

  const liveRel = [
    { label: "Part of", target: "Thesis writeup", target_id: "p-thesis" },
    { label: "Part of", target: "Reading list", target_id: "p-reading" },
  ];
  const pastRel = [{ label: "Part of", target: "Proposal draft", target_id: "p-proposal" }];

  const MODIFIED = 1782300000;
  const CREATED = 1781000000;
  const base = (over: Record<string, unknown>) => ({
    conventional: {
      kind: "file",
      size: 48213,
      mode: 0o644,
      modified_unix: MODIFIED,
      created_unix: CREATED,
      ...over,
    },
    woher: [
      { label: "Accessed by", detail: "Files" },
      { label: "Also accessed by", detail: "another app" },
    ],
    // A read that produced nothing, and says so as an outcome rather than an
    // empty list. Each fixture below overrides it with a different state, so one
    // screenshot shows all three: rows, refused, absent.
    verwandt: { state: "rows", rows: [] as typeof liveRel },
    zugriff: { readable_by: [] as string[], manage_link: "settings:ai" },
  });

  const fileInfo = { ...base({}), verwandt: { state: "rows", rows: liveRel } };
  const imageInfo = base({ kind: "file", size: 2_517_000 });
  // The two states an empty list used to swallow. A folder whose graph read was
  // refused must not read as "belongs to nothing", and a symlink on a machine with
  // no graph daemon must not either - so the fixtures differ deliberately.
  const folderInfo = {
    ...base({ kind: "directory", size: 0, mode: 0o755 }),
    woher: [],
    verwandt: { state: "denied", reason: "read scope" },
  };
  const symlinkInfo = {
    ...base({ kind: "symlink", mode: 0o777 }),
    woher: [],
    verwandt: { state: "unavailable", reason: "graph unreachable" },
  };

  let ready = $state(false);
  onMount(() => {
    // The op-error line has no data of its own: it renders whatever the last
    // failed action put there, so the harness puts one there.
    //
    // A KEY, not a sentence. This held an English literal until the store started
    // carrying keys, at which point the harness would have rendered the sentence
    // as a lookup and drawn the sentence itself back - a render harness quietly
    // showing something the app can no longer produce.
    opError.set({ key: "f.op.refused" });
  });

  onMount(async () => {
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd, args) => {
        const a = (args ?? {}) as Record<string, unknown>;
        if (cmd === "files_info") {
          if (a.path === IMAGE) return imageInfo;
          if (a.path === FOLDER) return folderInfo;
          if (a.path === SYMLINK) return symlinkInfo;
          return fileInfo;
        }
        // The live shape the Rust `provenance_of` (PH-R2) emits: a real,
        // graph-backed chain (mocked:false), every step `graph` origin, the access
        // step `pid` fidelity, a foreign co-tenant summarised at `proxy`, horizon
        // deeper_gated. Renders the NON-sample state so the serde contract and the
        // "no sample banner" branch are both exercised.
        if (cmd === "provenance_of" && a.ref === REALFILE)
          return {
            subject: "thesis-draft.md",
            steps: [
              { relation: "Part of", actor: "Thesis writeup", origin: "graph", when: "3 days ago", fidelity: "resolved" },
              { relation: "Last opened by", actor: "Files", origin: "graph", when: "2 hours ago", fidelity: "pid" },
              { relation: "Also opened by", actor: "another app", origin: "graph", when: "2 hours ago", fidelity: "proxy" },
            ],
            horizon: "deeper_gated",
            mocked: false,
            // A membership read that did not answer: the chain below is shorter
            // than the truth and nothing about its LENGTH shows that.
            incomplete: true,
          };
        if (cmd === "files_verwandt_as_of")
          return a.path === FILE
            ? { state: "rows", rows: pastRel }
            : { state: "rows", rows: [] };
        if (cmd === "files_get_exif_tags")
          return { description: "Sunset over the Inn", artist: "Tim", copyright: null };
        if (cmd === "files_set_permissions" || cmd === "files_set_exif_tags") return null;
        if (cmd === "files_thumbnail") return a.path === IMAGE ? thumbDataUri : null;
        throw new Error(`unmocked: ${cmd}`);
      });
    }
    ready = true;
  });

  const entry = (name: string, kind: string): FileEntry =>
    ({
      name,
      is_hidden: false,
      kind,
      size: 0,
      modified_unix: MODIFIED,
      readonly: false,
      symlink_target: null,
      full_path: null,
      restore_token: null,
    }) as unknown as FileEntry;
</script>

<div class="harness">
  {#if ready}
    <div class="host">
      <h2>File: related + as-of</h2>
      <FmInfoPanel path={FILE} entry={entry("thesis-draft.md", "file")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Image: EXIF + permissions</h2>
      <FmInfoPanel path={IMAGE} entry={entry("inn-sunset.jpg", "file")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Folder</h2>
      <FmInfoPanel path={FOLDER} entry={entry("Projects", "directory")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Symlink</h2>
      <FmInfoPanel path={SYMLINK} entry={entry("shortcut", "symlink")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Provenance halo (no backend)</h2>
      <!-- `provenance_of` is deliberately NOT mocked for this ref, so the store
           takes its fixture fallback - the state a session is in when the backend
           is absent. The halo must declare it as sample data. -->
      <ProvenanceHalo fileRef={FILE} />
    </div>
    <!-- A stable hook for the screenshot harness. There are six `.ph-trigger`s on
         this page and `--open` takes the first, which is an info panel's fixture -
         a positional selector would silently photograph the wrong chain. -->
    <div class="host" data-shot="live-halo">
      <h2>Provenance halo (live shape, PH-R2)</h2>
      <!-- `provenance_of` IS mocked for REALFILE with the exact shape the Rust
           backend emits: a real graph-backed chain, no sample banner. -->
      <ProvenanceHalo fileRef={REALFILE} />
    </div>
  {/if}

  <!-- The status bar for a VIRTUAL location, which is the other place an empty
       list used to be the only answer. "0 items" under a project the graph could
       not be asked about is a count nobody measured. -->
  <section class="bars">
    <h2>Status bar: an empty location vs one that could not be read</h2>
    <FmStatusBar entries={[]} selected={[]} />
    <FmStatusBar entries={[]} selected={[]} readReason="denied" />
    <FmStatusBar entries={[]} selected={[]} readReason="unavailable" />
  </section>
  <!-- The op-error line, which is where a refused eject, a refused mount and a
       failed pin now land. Before, each of those was a click that changed nothing
       and said nothing. -->
  <section class="bars">
    <h2>Op-error line: an action that was refused</h2>
    <OpsOverlays />
  </section>
</div>

<style>
  .harness {
    display: flex;
    gap: 20px;
    padding: 24px;
    min-height: 100vh;
    background: var(--background);
  }
  .host {
    display: flex;
    flex-direction: column;
  }
  .bars {
    /* A column of the flex harness, wide enough that the reason is not wrapped
       into something a screenshot cannot read. */
    min-width: 320px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  h2 {
    margin: 0 0 8px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
