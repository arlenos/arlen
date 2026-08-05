<script lang="ts">
  import { t } from "$lib/i18n/messages";
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
        {$t("v.playPause")}
        <ContextMenu.Shortcut>Space</ContextMenu.Shortcut>
      </ContextMenu.Item>
      <ContextMenu.Separator />
      <ContextMenu.Item>
        {$t("v.nextFile")}
        <ContextMenu.Shortcut>→</ContextMenu.Shortcut>
      </ContextMenu.Item>
      <ContextMenu.Item>
        {$t("v.prevFile")}
        <ContextMenu.Shortcut>←</ContextMenu.Shortcut>
      </ContextMenu.Item>

      {#if kind === "video"}
        <ContextMenu.Item>
          {$t("v.fullscreen")}
          <ContextMenu.Shortcut>F</ContextMenu.Shortcut>
        </ContextMenu.Item>
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>{$t("v.audioTrack")}</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-52">
            <!-- Placeholder track list: these names are hardcoded, not read from the
                 file. Left untranslated so they do not read as a real track listing. -->
            <ContextMenu.RadioGroup bind:value={audioTrack}>
              <ContextMenu.RadioItem value="0">English (stereo)</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="1">Commentary</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>{$t("v.subtitles")}</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-52">
            <!-- Same: the language entries below are sample data, not the file's
                 actual subtitle tracks. Only "Off" is real UI copy. -->
            <ContextMenu.RadioGroup bind:value={subtitles}>
              <ContextMenu.RadioItem value="off">{$t("v.off")}</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="en">English</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="de">German</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="srt">film.en.srt</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
            <ContextMenu.Separator />
            <ContextMenu.Item>{$t("v.loadSubtitleFile")}</ContextMenu.Item>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
      {/if}

      {#if kind === "audio"}
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>{$t("v.repeat")}</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-44">
            <ContextMenu.RadioGroup bind:value={repeat}>
              <ContextMenu.RadioItem value="off">{$t("v.off")}</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="file">{$t("v.repeatThisFile")}</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="folder">{$t("v.repeatFolder")}</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
        <ContextMenu.CheckboxItem bind:checked={shuffle}>{$t("v.shuffleFolder")}</ContextMenu.CheckboxItem>
        <ContextMenu.CheckboxItem bind:checked={autoplay}>{$t("v.autoplayNext")}</ContextMenu.CheckboxItem>
      {/if}

      <ContextMenu.Sub>
        <ContextMenu.SubTrigger>{$t("v.playbackSpeed")}</ContextMenu.SubTrigger>
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
          <ContextMenu.SubTrigger>{$t("v.loop")}</ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="w-44">
            <ContextMenu.RadioGroup bind:value={loop}>
              <ContextMenu.RadioItem value="off">{$t("v.off")}</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="file">{$t("v.repeatThisFile")}</ContextMenu.RadioItem>
              <ContextMenu.RadioItem value="folder">{$t("v.repeatFolder")}</ContextMenu.RadioItem>
            </ContextMenu.RadioGroup>
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
        <ContextMenu.Separator />
        <ContextMenu.Item>
          {$t("v.snapshotFrame")}
          <ContextMenu.Shortcut>S</ContextMenu.Shortcut>
        </ContextMenu.Item>
      {/if}

      <ContextMenu.Separator />
    {/if}

    <ContextMenu.Item>
      {$t("v.details")}
      <ContextMenu.Shortcut>I</ContextMenu.Shortcut>
    </ContextMenu.Item>
    <ContextMenu.Separator />
    <ContextMenu.Item>{$t("v.openWith")}</ContextMenu.Item>
    <ContextMenu.Item>{$t("v.showInFiles")}</ContextMenu.Item>
    <ContextMenu.Item>{$t("v.copy")}</ContextMenu.Item>
  </ContextMenu.Content>
</ContextMenu.Root>
