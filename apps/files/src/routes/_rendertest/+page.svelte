<script lang="ts">
  /// Headless render harness for FmInfoPanel — UI-AFFORDANCE verification ONLY,
  /// NOT a behaviour claim. The real provenance / as-of *behaviour* is proven by
  /// the coder's KG seed + integration tests; this route mocks the daemon over
  /// Tauri IPC so the affordances are screenshot-verifiable without a daemon:
  /// the clickable Related rows + hover, the "As of" control, the "Past view,
  /// as of …" cue, and the empty state. Not shipped in any nav; a dev/test route
  /// only (the mock is installed on mount and only when no Tauri runtime is
  /// present, so it can never hijack the real app).
  import { onMount } from "svelte";
  import { tauriAvailable } from "$lib/tauri";
  import FmInfoPanel from "$lib/components/FmInfoPanel.svelte";
  import type { FileEntry } from "@arlen/ui-kit/components/browser";

  // Two scenarios keyed by path: one whose membership changed over time (as-of
  // returns a different project), one with no past membership (the empty state).
  const WITH_HISTORY = "/demo/thesis-draft.md";
  const NO_HISTORY = "/demo/scratch.txt";

  const liveRel = [
    { label: "Part of project", target: "Thesis writeup", target_id: "p-thesis" },
    { label: "Part of project", target: "Reading list", target_id: "p-reading" },
  ];
  const pastRel = [
    { label: "Part of project", target: "Proposal draft", target_id: "p-proposal" },
  ];

  const fixtureInfo = {
    conventional: { kind: "file", size: 48213, mode: 0o644, modified_unix: 1782300000 },
    woher: [
      { label: "Downloaded", detail: "from share.uni-innsbruck.ac.at" },
      { label: "Created", detail: "2 weeks ago" },
    ],
    verwandt: liveRel,
    zugriff: { readable_by: ["you", "backup"], manage_link: "settings:ai" },
  };

  let ready = $state(false);
  onMount(async () => {
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd, args) => {
        const a = args as Record<string, unknown> | undefined;
        if (cmd === "files_info") return fixtureInfo;
        // Past membership: the with-history file was in a different project then;
        // the other had none (drives the empty state).
        if (cmd === "files_verwandt_as_of") return a?.path === WITH_HISTORY ? pastRel : [];
        // Anything else the panel probes (exif, etc.): reject so it hits .catch.
        throw new Error(`unmocked: ${cmd}`);
      });
    }
    ready = true;
  });

  const entry = (name: string): FileEntry =>
    ({
      name,
      is_hidden: false,
      kind: "file",
      size: 48213,
      modified_unix: 1782300000,
      readonly: false,
      symlink_target: null,
      full_path: null,
      restore_token: null,
    }) as unknown as FileEntry;
</script>

<div class="harness">
  {#if ready}
    <div class="host">
      <h2>With past membership</h2>
      <FmInfoPanel path={WITH_HISTORY} entry={entry("thesis-draft.md")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>No past membership (empty state)</h2>
      <FmInfoPanel path={NO_HISTORY} entry={entry("scratch.txt")} onnavigate={() => {}} />
    </div>
  {/if}
</div>

<style>
  .harness {
    display: flex;
    gap: 24px;
    padding: 24px;
    min-height: 100vh;
    background: var(--background);
  }
  .host {
    display: flex;
    flex-direction: column;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
