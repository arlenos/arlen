<script lang="ts">
  /// A navigation card linking to a sub-page: a leading icon, a title, a quiet
  /// description, and a trailing chevron, on the flat house style. The whole
  /// card is the link. Use it for "go to this related surface" affordances
  /// (the Settings AI page's provider/model links, keyboard -> shortcuts, …)
  /// instead of hand-rolling an `<a>` per page.
  import { ChevronRight } from "@lucide/svelte";
  import type { Snippet } from "svelte";

  let {
    href,
    title,
    description,
    icon,
    external = false,
  }: {
    /// The destination. In-app routes navigate via the router; external links
    /// open normally.
    href: string;
    title: string;
    /// The quiet second line; often live info (e.g. "3 connected").
    description?: string;
    /// The leading icon, as a snippet (package-agnostic across lucide builds):
    /// `{#snippet icon()}<Cloud size={20} />{/snippet}`.
    icon?: Snippet;
    /// Marks the link as leaving the app (adds rel/target).
    external?: boolean;
  } = $props();
</script>

<a
  class="link-card span-full"
  {href}
  target={external ? "_blank" : undefined}
  rel={external ? "noopener noreferrer" : undefined}
>
  {#if icon}
    <span class="lc-icon" aria-hidden="true">{@render icon()}</span>
  {/if}
  <span class="lc-body">
    <span class="lc-title">{title}</span>
    {#if description}<span class="lc-desc">{description}</span>{/if}
  </span>
  <ChevronRight class="lc-chev" size={16} strokeWidth={2} />
</a>

<style>
  .link-card {
    display: flex;
    align-items: center;
    gap: 0.875rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius-card);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    /* Card surface + elevation per theme (white lift on light, lighter card on
       dark); the proven Group pattern. */
    background: var(--card, color-mix(in srgb, var(--foreground) 3%, transparent));
    box-shadow: var(--shadow-card, none);
    text-decoration: none;
    color: inherit;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .link-card:hover {
    background: color-mix(in srgb, var(--foreground) 6%, var(--card, transparent));
  }
  .lc-icon {
    flex-shrink: 0;
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .lc-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .lc-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .lc-desc {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.lc-chev) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    transition: transform var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .link-card:hover :global(.lc-chev) {
    transform: translateX(2px);
  }
</style>
