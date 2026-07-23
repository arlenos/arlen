<script lang="ts">
  /// The Knowledge places sidebar (knowledge-app.md §2): the explore places over the
  /// graph, plus a Capabilities row that links out to Settings/Privacy (decision 6 -
  /// the capability browser is not re-hosted here). A fixed nav list, so it is its
  /// own light component rather than the file-oriented kit PlacesSidebar.
  import { Clock, FolderGit2, Search, Library, Package, ShieldCheck, ChevronRight } from "lucide-svelte";
  import { t } from "$lib/i18n/messages";

  let {
    activeLocation,
    onnavigate,
    oncapabilities,
  }: {
    activeLocation: string;
    onnavigate: (location: string) => void;
    oncapabilities: () => void;
  } = $props();

  // The explore places with their icons, in sidebar order (§2). The label/empty
  // presentation lives in `locations.ts`; the icon pairing lives here.
  const PLACES = [
    { id: "timeline", labelKey: "k.place.timeline", icon: Clock },
    { id: "projects", labelKey: "k.place.projects", icon: FolderGit2 },
    { id: "searches", labelKey: "k.place.searches", icon: Search },
    { id: "library", labelKey: "k.place.library", icon: Library },
    { id: "capsules", labelKey: "k.place.capsules", icon: Package },
  ];

  function isActive(id: string): boolean {
    const scheme = activeLocation.split(":")[0] ?? activeLocation;
    if (id === activeLocation) return true;
    if (id === "searches" && scheme === "search") return true;
    if (id === "projects" && scheme === "project") return true;
    return false;
  }
</script>

<nav class="kn-side" aria-label={$t("k.title")}>
  <div class="kn-side-title">{$t("k.title")}</div>

  <div class="kn-group">{$t("k.section.explore")}</div>
  {#each PLACES as p (p.id)}
    {@const Icon = p.icon}
    <button
      type="button"
      class="kn-place"
      class:active={isActive(p.id)}
      aria-current={isActive(p.id) ? "page" : undefined}
      onclick={() => onnavigate(p.id)}
    >
      <Icon size={16} strokeWidth={1.75} />
      <span>{$t(p.labelKey)}</span>
    </button>
  {/each}

  <div class="kn-group">{$t("k.section.authority")}</div>
  <button type="button" class="kn-place kn-linkout" onclick={oncapabilities}>
    <ShieldCheck size={16} strokeWidth={1.75} />
    <span>{$t("k.place.capabilities")}</span>
    <span class="kn-caret"><ChevronRight size={14} strokeWidth={2} /></span>
  </button>
  <span class="kn-linkout-note">{$t("k.caps.opens")}</span>
</nav>

<style>
  .kn-side {
    flex: 0 0 15rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.75rem 0.6rem;
    border-right: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow-y: auto;
  }
  .kn-side-title {
    padding: 0.25rem 0.5rem 0.6rem;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .kn-group {
    padding: 0.6rem 0.5rem 0.25rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
  .kn-place {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    padding: 0.4rem 0.5rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 82%, transparent);
    cursor: pointer;
  }
  .kn-place:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .kn-place.active {
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    color: var(--color-fg-primary);
    font-weight: 500;
  }
  .kn-linkout {
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .kn-caret {
    margin-inline-start: auto;
    display: inline-flex;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .kn-linkout-note {
    padding: 0.05rem 0.5rem 0;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
</style>
