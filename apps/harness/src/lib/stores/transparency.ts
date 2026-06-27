/// Whether the transparency drawer is open. The drawer is the single
/// accountability surface (it replaced the standalone /transparency page);
/// it is summoned from the composer foot and mounted once in the layout, so
/// its open state is a shared store rather than route state.
import { writable } from "svelte/store";

/// True while the transparency drawer is shown over the conversation.
export const transparencyOpen = writable(false);

/// Open the transparency drawer (from the composer foot or a deep link).
export function openTransparency(): void {
  transparencyOpen.set(true);
}
