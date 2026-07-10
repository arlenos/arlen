<script lang="ts">
  /// An artifact rendered inline in the assistant turn: a quiet title over the
  /// content (no toolbar). Actions live in a right-click context menu (the
  /// desktop convention), not visible buttons: Copy works now; Save to file and
  /// Pin are coder Tauri/KG seams and render disabled (greyed) until wired, the
  /// desktop convention for an unavailable action rather than a fake one.
  import { t } from "$lib/i18n/messages";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";
  import ArtifactView from "$lib/components/artifact/ArtifactView.svelte";
  import { kindLabel, type Artifact } from "$lib/components/artifact/types";

  let {
    artifact,
    onsave,
    onpin,
  }: { artifact: Artifact; onsave?: () => void; onpin?: () => void } = $props();

  const title = $derived(artifact.meta.title ?? kindLabel(artifact.kind));

  async function copy() {
    try {
      await navigator.clipboard.writeText(artifact.text);
    } catch {
      // clipboard may be unavailable; nothing to surface for a copy
    }
  }
</script>

<ContextMenu.Root>
  <ContextMenu.Trigger class="ab-trigger">
    <figure class="ab">
      <figcaption class="ab-title">{title}</figcaption>
      <ArtifactView {artifact} />
    </figure>
  </ContextMenu.Trigger>
  <ContextMenu.Content class="w-52">
    <ContextMenu.Item onclick={copy}>{$t("h.artifact.copy")}</ContextMenu.Item>
    <ContextMenu.Item onclick={onsave} disabled={!onsave}>{$t("h.artifact.save")}</ContextMenu.Item>
    <ContextMenu.Item onclick={onpin} disabled={!onpin}>{$t("h.artifact.pin")}</ContextMenu.Item>
  </ContextMenu.Content>
</ContextMenu.Root>

<style>
  :global(.ab-trigger) {
    display: block;
    width: 100%;
    text-align: left;
  }
  .ab {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }
  .ab-title {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
