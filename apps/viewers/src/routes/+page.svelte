<script lang="ts">
  /// The viewer routes one window to one file by media type. While the decode
  /// backend is wired separately, a `?demo=` query selects which face to render
  /// against mock data (the screenshot harness drives each one). `?w=&h=` size
  /// a fixed window so a headless full-page shot is exactly that window (the
  /// WebDriver ignores window/rect and the viewport varies under Xvfb).
  import { page } from "$app/state";
  import AudioPlayer from "$lib/components/AudioPlayer.svelte";
  import ImageViewer from "$lib/components/ImageViewer.svelte";
  import { audioMock, imageMock } from "$lib/mock";

  let demo = $derived(page.url.searchParams.get("demo") ?? "audio");
  let w = $derived(Number(page.url.searchParams.get("w")));
  let h = $derived(Number(page.url.searchParams.get("h")));
  let framed = $derived(!!page.url.searchParams.get("w") && !!page.url.searchParams.get("h"));
</script>

{#snippet face(d: string)}
  {#if d === "audio"}
    <AudioPlayer file={audioMock} />
  {:else if d === "image"}
    <ImageViewer file={imageMock} />
  {/if}
{/snippet}

{#if framed}
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
</style>
