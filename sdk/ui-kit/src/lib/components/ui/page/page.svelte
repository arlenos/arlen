<script lang="ts">
  /// Canonical settings/app page wrapper: padded content region with an
  /// optional standard header (title + description) and, on a sub-route, the
  /// door back to its parent. It does NOT own a scroll container — the app
  /// shell's content region already scrolls; Page only provides the page
  /// padding and header. Width + reflow are the job of `SectionGrid`, which
  /// Page centres by sharing the same max-width on its header. See
  /// `docs/architecture/design-system.md` §5 and §6.9.
  ///
  /// The door is a kit affordance, not a per-page one: a route a person can
  /// reach carries a visible, keyboard-reachable way back to where they came
  /// from, in the page header. It is an anchor (the router intercepts it, as
  /// with `LinkCard`), and when the previous page was the parent it steps back
  /// through history instead so the parent keeps its scroll and its state. A
  /// top-level panel passes no `back`: the sidebar is its way out.
  import { onMount, type Snippet } from "svelte";
  import { ChevronLeft } from "@lucide/svelte";
  import { kt } from "../../../i18n/messages.kit";
  import { cameFrom, notePage } from "./history";

  let {
    title,
    description,
    back,
    children,
  }: {
    title?: string;
    description?: string;
    /// The parent this page was entered from: its route and the word the
    /// person knows it by (the same word the navigation uses).
    back?: { href: string; label: string };
    children?: Snippet;
  } = $props();

  onMount(() => {
    notePage(window.location.pathname);
  });

  function goBack(event: MouseEvent) {
    if (!back || !cameFrom(back.href)) return;
    event.preventDefault();
    history.back();
  }
</script>

<div class="page">
  {#if title || back}
    <header class="page-header">
      {#if back}
        <a class="page-back" href={back.href} aria-label={$kt("k.page.back", { name: back.label })} onclick={goBack}>
          <ChevronLeft size={14} strokeWidth={2} aria-hidden="true" />
          <span>{back.label}</span>
        </a>
      {/if}
      {#if title}
        <h1 class="page-title">{title}</h1>
      {/if}
      {#if description}
        <p class="page-desc">{description}</p>
      {/if}
    </header>
  {/if}
  {@render children?.()}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    /* Use the spacing tokens so apps can override in :root without
       hunting for literal values across components. */
    gap: var(--space-section, 1.5rem);
    padding: var(--space-page, 1.5rem);
  }

  /* The header shares the grid's max-width + centring so the title lines up
     with the left edge of the section cards. */
  .page-header {
    width: 100%;
    max-width: var(--width-section-body, 46rem);
    margin-inline: auto;
  }

  /* The door: a compact control in the kit's button family, sitting above the
     title where a person looks for the way back. Pulled left by its own
     padding so its text aligns with the title's edge. */
  .page-back {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    height: var(--height-control-compact, 26px);
    margin-inline-start: -0.5rem;
    margin-bottom: 0.5rem;
    padding-inline: 0.5rem 0.625rem;
    border-radius: var(--radius-button);
    font-size: var(--text-sm);
    line-height: 1;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
    text-decoration: none;
    transition:
      color var(--duration-fast, 120ms) var(--ease-default, ease),
      background-color var(--duration-fast, 120ms) var(--ease-default, ease);
  }
  .page-back:hover {
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .page-back:focus-visible {
    outline: 2px solid var(--ring, var(--foreground));
    outline-offset: 1px;
  }

  .page-title {
    font-size: var(--text-xl);
    font-weight: 600;
    line-height: 1.2;
    color: var(--foreground);
  }

  .page-desc {
    margin-top: 0.25rem;
    font-size: var(--text-sm);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
