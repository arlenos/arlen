<script lang="ts">
  /// Filesystem directory picker.
  ///
  /// Renders as a single Button. On click, invokes the backend
  /// `pick_directory` Tauri command which shells out to the system
  /// file chooser (zenity / kdialog / xdg-desktop-portal). Returns
  /// the chosen path via `onpick`, or stays silent if the user
  /// cancelled.
  ///
  /// Used by Focus Mode + Knowledge Graph pages for the
  /// "watch directories" lists. Wrapped this way because we don't
  /// have a Tauri-plugin-dialog dep and the cross-DE story for
  /// modal pickers is "pick whatever the system has".

  import { invoke } from "@tauri-apps/api/core";
  import { FolderOpen } from "lucide-svelte";
  import { Button } from "$lib/components/ui/button";

  let {
    onpick,
    startPath,
    label = "Choose Folder",
    disabled = false,
    variant = "outline",
    size = "sm",
  }: {
    /// Called with the picked path (absolute) when the user confirms.
    /// Not called when the user cancels — callers don't need to handle
    /// a null branch.
    onpick: (path: string) => void;
    /// Optional starting directory. Defaults to `$HOME`.
    startPath?: string;
    label?: string;
    disabled?: boolean;
    variant?: "outline" | "ghost" | "default";
    size?: "sm" | "default";
  } = $props();

  let pending = $state(false);

  async function pick() {
    if (pending || disabled) return;
    pending = true;
    try {
      const path = await invoke<string | null>("pick_directory", {
        startPath: startPath ?? null,
      });
      if (path !== null && path.length > 0) {
        onpick(path);
      }
    } catch (err) {
      console.warn("pick_directory failed:", err);
    } finally {
      pending = false;
    }
  }
</script>

<Button {variant} {size} disabled={disabled || pending} onclick={pick}>
  <FolderOpen size={14} />
  {label}
</Button>
