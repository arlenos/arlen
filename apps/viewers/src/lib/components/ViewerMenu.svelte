<script lang="ts">
  /// The viewer's right-click menu (quickview-plan.md "The menus"), per file
  /// type, text-only on the flat @arlen/ui-kit ContextMenu canon (matching the
  /// FM folder right-click - no icons, no web/native menu). Wraps the viewer
  /// surface as the trigger; the depth (Details/tracks/subtitles) lives here so
  /// nothing is a fixed button in the window. Handlers are optional - the host
  /// wires them; the demo leaves them no-ops. Audio-track/subtitles/speed/loop
  /// are single-select submenus, autoplay/shuffle are toggles.
  import type { Snippet } from "svelte";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";

  let {
    kind,
    open = $bindable(false),
    children,
  }: {
    /// Which face's menu to show.
    kind: "image" | "video" | "audio";
    /// Bindable open state (the demo forces it; right-click drives it normally).
    open?: boolean;
    /// The viewer surface the menu anchors to (the right-click target).
    children?: Snippet;
  } = $props();

  // Demo state for the toggles + radios so the rendered menu shows real marks.
  let repeat = $state("off");
  let loop = $state("off");
  let speed = $state("1");
  let audioTrack = $state("0");
  let subtitles = $state("off");
  let shuffle = $state(false);
  let autoplay = $state(true);

  const SPEEDS = [
    ["0.5", "0.5×"],
    ["0.75", "0.75×"],
    ["1", "Normal"],
    ["1.25", "1.25×"],
    ["1.5", "1.5×"],
    ["2", "2×"],
  ];
</script>

<ContextMenu.Root bind:open>
  <ContextMenu.Trigger>
    {@render children?.()}
  </ContextMenu.Trigger>
  <ContextMenu.Content class="w-56">
    {#if kind !== "image"}
      <ContextMenu.Item>
        Play / Pause
        <ContextMenu.Shortcut>Space</ContextMenu.Shortcut>
      </ContextMenu.Item>
      <ContextMenu.Separator />
      <ContextMenu.Item>
        Next file
        <ContextMenu.Shortcut>→</ContextMenu.Shortcut>
      </ContextMenu.Item>
      <ContextMenu.Item>
        Previous file
        <ContextMenu.Shortcut>←</ContextMenu.Shortcut>
      </ContextMenu.Item>

      {#if kind === "video"}
        <ContextMenu.Item>
          Fullscreen
          <ContextMenu.Shortcut>F</ContextMenu.Shortcut>
        </ContextMenu.Item>
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>Audio track</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-52">
            <ContextMenu.RadioGroup bind:value={audioTrack}>
              <ContextMenu.RadioItem value="0">English (stereo)</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="1">Commentary</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>Subtitles</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-52">
            <ContextMenu.RadioGroup bind:value={subtitles}>
              <ContextMenu.RadioItem value="off">Off</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="en">English</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="de">German</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="srt">film.en.srt</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
            <ContextMenu.Separator />
            <ContextMenu.Item>Load subtitle file…</ContextMenu.Item>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
      {/if}

      {#if kind === "audio"}
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>Repeat</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-44">
            <ContextMenu.RadioGroup bind:value={repeat}>
              <ContextMenu.RadioItem value="off">Off</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="file">This file</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="folder">Folder</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
        <ContextMenu.CheckboxItem bind:checked={shuffle}>Shuffle folder</ContextMenu.CheckboxItem>
        <ContextMenu.CheckboxItem bind:checked={autoplay}>Autoplay next</ContextMenu.CheckboxItem>
      {/if}

      <ContextMenu.Sub>
        <ContextMenu.SubTrigger>Playback speed</ContextMenu.SubTrigger>
        <ContextMenu.SubContent class="w-40">
          <ContextMenu.RadioGroup bind:value={speed}>
            {#each SPEEDS as [v, label] (v)}
              <ContextMenu.RadioItem value={v}>{label}</ContextMenu.RadioItem>
            {/each}
          </ContextMenu.RadioGroup>
        </ContextMenu.SubContent>
      </ContextMenu.Sub>

      {#if kind === "video"}
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>Loop</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-44">
            <ContextMenu.RadioGroup bind:value={loop}>
              <ContextMenu.RadioItem value="off">Off</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="file">This file</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="folder">Folder</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
        <ContextMenu.Separator />
        <ContextMenu.Item>
          Snapshot frame
          <ContextMenu.Shortcut>S</ContextMenu.Shortcut>
        </ContextMenu.Item>
      {/if}

      <ContextMenu.Separator />
    {/if}

    <ContextMenu.Item>
      Details…
      <ContextMenu.Shortcut>I</ContextMenu.Shortcut>
    </ContextMenu.Item>
    <ContextMenu.Separator />
    <ContextMenu.Item>Open with…</ContextMenu.Item>
    <ContextMenu.Item>Show in Files</ContextMenu.Item>
    <ContextMenu.Item>Copy</ContextMenu.Item>
  </ContextMenu.Content>
</ContextMenu.Root>
