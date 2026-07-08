<script lang="ts">
  /// Top bar auto-show status badge.
  ///
  /// Compact icon (+ optional inline label) shown in the top bar
  /// only while the underlying state is active. Used for caffeine,
  /// recording, night-light, airplane mode, focus session.
  ///
  /// Visibility is controlled entirely by the caller via `visible`.
  /// The badge fades in/out on transition; the parent slot collapses
  /// the layout when no badges are visible (TopBar.svelte handles
  /// the row-spacing logic — this component only renders one slot).
  ///
  /// `onclick` is the canonical "tap to interact" entry point. For
  /// caffeine/recording it cycles state; for night-light/airplane it
  /// toggles; for focus it opens the project flyout (orchestrator's
  /// choice).
  import type { Snippet } from "svelte";

  let {
    icon,
    label = "",
    visible,
    active = true,
    title = "",
    pulsate = false,
    onclick,
  }: {
    /// Icon snippet (e.g. `<Coffee size={14} />`).
    icon: Snippet;
    /// Optional inline label rendered to the right of the icon.
    /// Empty string hides the label entirely (icon-only badge).
    label?: string;
    /// Whether the badge is mounted at all. The parent slot reads
    /// this for layout purposes.
    visible: boolean;
    /// `true` paints in accent / status colour; `false` falls back to
    /// muted foreground. Almost all badges are active when visible —
    /// the prop exists for the rare "show ambient" case.
    active?: boolean;
    /// `title` attribute (native tooltip). Component does not own the
    /// shadcn Tooltip — wrap externally if a styled one is needed.
    title?: string;
    /// `true` adds a slow pulse animation. Used for the Recording
    /// badge to draw the eye.
    pulsate?: boolean;
    /// Click handler. The component is interactive only if a handler
    /// is provided.
    onclick?: () => void;
  } = $props();
</script>

{#if visible}
  {#if onclick}
    <button
      type="button"
      class="status-badge"
      class:active
      class:pulsate
      class:has-label={label.length > 0}
      {title}
      onclick={(e) => {
        e.stopPropagation();
        onclick();
      }}
    >
      <span class="status-badge-icon">{@render icon()}</span>
      {#if label}
        <span class="status-badge-label">{label}</span>
      {/if}
    </button>
  {:else}
    <span
      class="status-badge"
      class:active
      class:pulsate
      class:has-label={label.length > 0}
      {title}
    >
      <span class="status-badge-icon">{@render icon()}</span>
      {#if label}
        <span class="status-badge-label">{label}</span>
      {/if}
    </span>
  {/if}
{/if}

<style>
  .status-badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 6px;
    border-radius: var(--radius-chip);
    background: transparent;
    border: none;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: default;
    font-size: 0.6875rem;
    line-height: 1;
    transition: background-color 100ms ease, color 100ms ease;
  }
  .status-badge.has-label {
    padding-inline-end: 8px;
  }
  button.status-badge:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
  .status-badge.active {
    color: var(--color-accent);
  }
  .status-badge-icon {
    display: inline-flex;
  }
  .status-badge-label {
    font-variant-numeric: tabular-nums;
  }
  .status-badge.pulsate {
    animation: status-badge-pulse 1.6s ease-in-out infinite;
  }
  @keyframes status-badge-pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.55; }
  }
</style>
