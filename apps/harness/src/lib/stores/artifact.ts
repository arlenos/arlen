/// The artifact currently shown in the right pane (or null when the pane is
/// closed). A pane artifact's chat card opens it; a freshly produced pane
/// artifact opens it automatically; closing it clears this.
import { writable } from "svelte/store";
import type { Artifact } from "$lib/components/artifact/types";

export const openArtifact = writable<Artifact | null>(null);

/// Open the right pane on an artifact.
export function openPane(artifact: Artifact): void {
  openArtifact.set(artifact);
}

/// Close the right pane.
export function closePane(): void {
  openArtifact.set(null);
}
