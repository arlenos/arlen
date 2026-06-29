<script lang="ts">
  /// The shared right-click menu for file-reference pills (mounted once near the
  /// transcript). The kit `DropdownMenu` in controlled mode, floated against a
  /// virtual anchor at the cursor - so it reuses the kit's items, positioning,
  /// portal, outside-click + Escape + keyboard nav rather than hand-rolling a
  /// menu. Open / Open with / Reveal in Files route AS THE USER through the
  /// desktop opener (not the AI daemon); the copies are client-side. An
  /// unresolvable path offers only the copies.
  import { invoke } from "@tauri-apps/api/core";
  import { FolderOpen, AppWindow, FolderSearch, Copy } from "@lucide/svelte";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import { fileRefMenu, closeFileRefMenu } from "$lib/stores/fileRefMenu";

  // Collapse the home prefix to ~ for the "relative" copy; a true project root
  // needs backend context (flagged), so this is the short, portable form.
  function relativePath(path: string): string {
    const m = path.match(/^\/home\/[^/]+(\/.*)?$/);
    return m ? `~${m[1] ?? ""}` : path;
  }

  function run(cmd: string): void {
    const path = $fileRefMenu.path;
    closeFileRefMenu();
    invoke(cmd, { path }).catch((err) => console.error(`${cmd} failed`, err));
  }

  async function copy(text: string): Promise<void> {
    closeFileRefMenu();
    try {
      await navigator.clipboard.writeText(text);
    } catch (err) {
      console.error("clipboard write failed", err);
    }
  }

  // A zero-size virtual anchor at the cursor: the menu floats against this
  // instead of a trigger element, so a pill anywhere in the prose can summon it.
  const anchor = $derived({
    getBoundingClientRect: () => new DOMRect($fileRefMenu.x, $fileRefMenu.y, 0, 0),
  });
</script>

<DropdownMenu.Root
  open={$fileRefMenu.open}
  onOpenChange={(o) => {
    if (!o) closeFileRefMenu();
  }}
>
  <DropdownMenu.Content customAnchor={anchor} align="start" sideOffset={2} class="w-52">
    {#if $fileRefMenu.resolvable}
      <DropdownMenu.Item onSelect={() => run("fileref_open")}>
        <FolderOpen size={14} strokeWidth={2} />
        Open
      </DropdownMenu.Item>
      <DropdownMenu.Item onSelect={() => run("fileref_open_with")}>
        <AppWindow size={14} strokeWidth={2} />
        Open with…
      </DropdownMenu.Item>
      <DropdownMenu.Item onSelect={() => run("fileref_reveal")}>
        <FolderSearch size={14} strokeWidth={2} />
        Reveal in Files
      </DropdownMenu.Item>
      <DropdownMenu.Separator />
    {/if}
    <DropdownMenu.Item onSelect={() => copy($fileRefMenu.path)}>
      <Copy size={14} strokeWidth={2} />
      Copy path
    </DropdownMenu.Item>
    <DropdownMenu.Item onSelect={() => copy(relativePath($fileRefMenu.path))}>
      <Copy size={14} strokeWidth={2} />
      Copy relative path
    </DropdownMenu.Item>
  </DropdownMenu.Content>
</DropdownMenu.Root>
