<script lang="ts">
  /// A small standard About surface every first-party app reuses for Help >
  /// About: the app's mark + name, the Arlen identity and version, an optional
  /// one-line description. A system surface on the shared modal shell, not a
  /// web-style modal. The host passes its own name/version/description.
  import { Dialog } from "../dialog";
  import { Button } from "../button";

  type Props = {
    /// Whether the dialog is open.
    open: boolean;
    /// Close the dialog.
    onClose: () => void;
    /// The app's display name (e.g. "Files").
    appName: string;
    /// The app version string (e.g. "0.1.0").
    version: string;
    /// An optional one-line description of the app.
    description?: string;
  };

  let { open, onClose, appName, version, description }: Props = $props();
</script>

<Dialog {open} {onClose} ariaLabel={`About ${appName}`} size="sm">
  <div class="about">
    <div class="about-mark" aria-hidden="true">{appName.charAt(0)}</div>
    <div class="about-name">{appName}</div>
    <div class="about-sub">Arlen OS &middot; {version}</div>
    {#if description}
      <p class="about-desc">{description}</p>
    {/if}
    <div class="about-foot">
      <Button variant="ghost" onclick={onClose}>Close</Button>
    </div>
  </div>
</Dialog>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 4px;
    padding: 28px 24px 16px;
  }
  .about-mark {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 48px;
    height: 48px;
    margin-bottom: 8px;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
    font-size: var(--text-xl);
    font-weight: 600;
  }
  .about-name {
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--foreground);
  }
  .about-sub {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .about-desc {
    margin-top: 8px;
    max-width: 36ch;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .about-foot {
    margin-top: 16px;
    display: flex;
    justify-content: center;
  }
</style>
