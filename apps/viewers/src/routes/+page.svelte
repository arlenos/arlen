<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The viewer routes one window to one file by media type. When launched on a
  /// real file (`viewer <path>`, the `.desktop` `%f`, or a double-click) it loads
  /// it through the decode backend on mount; absent a real file it falls back to
  /// the mock `?demo=` path the screenshot harness drives. `?w=&h=` size a fixed
  /// window so a headless full-page shot is exactly that window.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/state";
  import AudioPlayer from "$lib/components/AudioPlayer.svelte";
  import ImageViewer from "$lib/components/ImageViewer.svelte";
  import VideoViewer from "$lib/components/VideoViewer.svelte";
  import { audioMock, imageMock, videoMock, mockPeaks, type AudioMock, type ImageMock } from "$lib/mock";
  import { tauriAvailable } from "$lib/tauri";

  let demo = $derived(page.url.searchParams.get("demo") ?? "audio");
  /// DEV: pin a failure state so it can be looked at. Both are reachable only
  /// from a real backend, so until this existed the two branches that carry the
  /// app's honesty - "could not open" and "nothing open" - were the only ones
  /// nobody had ever seen. The `noFile` comment below records that its defect was
  /// found by looking; this is what makes looking possible again.
  ///
  /// DEV-gated like the clock's `?nowake`: a shipped viewer must not be talkable
  /// into showing a failure that did not happen.
  const pinnedState = import.meta.env.DEV ? page.url.searchParams.get("state") : null;
  let w = $derived(Number(page.url.searchParams.get("w")));
  let h = $derived(Number(page.url.searchParams.get("h")));
  let framed = $derived(!!page.url.searchParams.get("w") && !!page.url.searchParams.get("h"));

  type Raster = { width: number; height: number; rgba: number[] };
  type AudioInfo = {
    codec: string;
    sample_rate: number;
    channels: number;
    duration_ms: number | null;
    title: string | null;
    artist: string | null;
    peaks: number[];
  };
  type Loaded =
    | { kind: "image"; file: ImageMock; raster: Raster }
    | { kind: "audio"; file: AudioMock };

  // The real file the window was opened on, decoded through the backend. `null`
  // keeps the mock/demo path (no Tauri runtime, or no file argument).
  let loaded = $state<Loaded | null>(null);
  let loadError = $state<string | null>(null);
  // Opened by the real shell with no file to show. Distinct from the demo path
  // below, and the distinction is the whole point: without it this window falls
  // back to the mock and a shipped viewer shows a track called "Nightswim" with
  // a waveform and a playhead at 1:13 of 3:40, none of which exists and none of
  // which says so. Found on 9 August in the first desktop-width sweep.
  let noFile = $state(false);

  /// Whether a failure message is machinery talking rather than something for a
  /// person. A decoder's own words are worth showing - "unsupported JPEG
  /// progressive scan" tells someone what is wrong with their file - but a JS
  /// runtime error names an internal and offers nothing to do about it.
  ///
  /// The same predicate as `readsAsInternal` in ui-kit's `FileBrowser`, which
  /// learned it by greeting a user with "TypeError: undefined is not an object
  /// (evaluating 'window.__TAURI_INTERNALS__.invoke')" in the middle of the pane.
  /// Copied rather than shared because it lives inside that component and the
  /// viewer does not depend on the browser module; if a third app needs it, that
  /// is the moment it moves somewhere both can reach.
  function readsAsInternal(message: string): boolean {
    return /\b(TypeError|ReferenceError|SyntaxError)\b|undefined is not|is not a function|window\.__/.test(
      message,
    );
  }

  function basename(p: string): string {
    return p.split("/").filter(Boolean).pop() ?? p;
  }

  onMount(async () => {
    if (pinnedState === "load-error") {
      loadError = "decode-image: unsupported JPEG progressive scan";
      return;
    }
    if (pinnedState === "internal-error") {
      // The half that has to be SUPPRESSED, so the guard can be seen working and
      // not just read.
      loadError = "TypeError: undefined is not an object (evaluating 'window.__TAURI_INTERNALS__.invoke')";
      return;
    }
    if (pinnedState === "no-file") {
      noFile = true;
      return;
    }
    if (!tauriAvailable) return;
    let path: string | null = null;
    try {
      path = await invoke<string | null>("initial_file");
    } catch {
      return; // no managed state / not the real shell - stay on the mock path
    }
    if (!path) {
      noFile = true;
      return;
    }
    const name = basename(path);
    try {
      const kind = await invoke<string>("detect_media_kind", { path });
      if (kind === "image") {
        const raster = await invoke<Raster>("decode_image", { path });
        loaded = { kind: "image", file: { name, index: 1, total: 1 }, raster };
      } else if (kind === "audio") {
        const info = await invoke<AudioInfo>("probe_audio", { path });
        loaded = {
          kind: "audio",
          file: {
            // Real tags from the probe, falling back to the file name for an
            // untagged file.
            title: info.title ?? name,
            artist: info.artist,
            codec: info.codec,
            durationSec: (info.duration_ms ?? 0) / 1000,
            // The real waveform from the probe's decode pass; the mock stands in
            // only when the track length is unknown or silent (empty peaks).
            peaks: info.peaks.length ? info.peaks : mockPeaks(),
            index: 1,
            total: 1,
          },
        };
      } else {
        loadError = `unsupported media kind: ${kind}`;
      }
    } catch (e) {
      loadError = String(e);
    }
  });
</script>

{#snippet face(d: string)}
  {#if d === "audio"}
    <AudioPlayer file={audioMock} />
  {:else if d === "image"}
    <ImageViewer file={imageMock} />
  {:else if d === "video"}
    <VideoViewer file={videoMock} />
  {/if}
{/snippet}

{#if loaded?.kind === "image"}
  <div class="fill"><ImageViewer file={loaded.file} raster={loaded.raster} /></div>
{:else if loaded?.kind === "audio"}
  <div class="fill"><AudioPlayer file={loaded.file} /></div>
{:else if loadError}
  <div class="fill err">
    {readsAsInternal(loadError)
      ? $t("v.couldNotOpenUnknown")
      : $t("v.couldNotOpen", { reason: loadError })}
  </div>
{:else if noFile}
  <!-- Before the demo branches on purpose: in the real shell an empty window is
       an empty window, and the sample below is for the harness and the browser. -->
  <div class="fill err">{$t("v.nothingOpen")}</div>
{:else if framed}
  <div class="frame" style="width:{w}px;height:{h}px">
    {@render face(demo)}
  </div>
{:else}
  <div class="fill">
    {@render face(demo)}
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
  }
  /* Mock-harness only: size the document to the window so a headless full-page
     screenshot is exactly the window. Never part of the product. */
  .frame {
    position: absolute;
    top: 0;
    left: 0;
    overflow: hidden;
  }
  .fill {
    width: 100vw;
    height: 100vh;
  }
  .err {
    display: grid;
    place-items: center;
    background: #0a0a0a;
    color: var(--color-fg-secondary, #a1a1aa);
    font-family: "Inter Variable", Inter, system-ui, sans-serif;
    font-size: 13px;
    padding: 24px;
    text-align: center;
  }
</style>
