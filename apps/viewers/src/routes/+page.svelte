<script lang="ts">
  import { printProblem, restoreProblem, trashProblem } from "$lib/trashProblem";
  import { t, locale } from "$lib/i18n/messages";
  import { formatSize } from "@arlen/ui-kit/components/browser";
  import { initAppMenu, menuAction } from "$lib/menu";
  /// The viewer routes one window to one file by media type. When launched on a
  /// real file (`viewer <path>`, the `.desktop` `%f`, or a double-click) it loads
  /// it through the decode backend on mount; absent a real file it falls back to
  /// the mock `?demo=` path the screenshot harness drives. `?w=&h=` size a fixed
  /// window so a headless full-page shot is exactly that window.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { page } from "$app/state";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import AudioPlayer from "$lib/components/AudioPlayer.svelte";
  import DetailsPanel, { type Fact } from "$lib/components/DetailsPanel.svelte";
  import ImageViewer from "$lib/components/ImageViewer.svelte";
  import VideoViewer from "$lib/components/VideoViewer.svelte";
  import { audioMock, imageMock, videoMock, type AudioMock, type ImageMock } from "$lib/mock";
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

  // The shell menu's dispatch, the same verbs the keys run.
  $effect(() => {
    const a = $menuAction;
    if (!a) return;
    menuAction.set(null);
    if (a === "file.print") void printCurrent();
    else if (a === "file.trash") void deleteCurrent();
    else if (a === "file.undo") void undoDelete();
    else if (a === "go.next") void step("next");
    else if (a === "go.previous") void step("previous");
  });

  onMount(async () => {
    void initAppMenu();
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
    } catch (e) {
      // Past the `tauriAvailable` guard above, this IS the real viewer window:
      // `__TAURI_INTERNALS__` is there from the moment the webview loads, so a
      // throw here is the backend failing to say which file was opened, not the
      // browser preview the mock exists for. Returning to the mock path put
      // "Nightswim" and a playhead at 1:13 of 3:40 into a shipped window - the
      // same defect the `noFile` comment above records, one branch over, and it
      // survived that fix because only the `!path` case was covered.
      // `readsAsInternal` keeps a runtime error from being quoted at a person.
      loadError = String(e);
      return;
    }
    if (!path) {
      noFile = true;
      return;
    }
    currentPath = path;
    await openFile(path);
  });

  /// Whether the details panel is open, and the facts behind it. Both are set
  /// only from what a decode, a probe or a stat actually returned - see
  /// `DetailsPanel` for why a fact nobody measured gets no row at all.
  let detailsOpen = $state(false);
  let facts = $state<Fact[]>([]);

  /// Bytes as a person reads them - `formatSize` from the kit, which is what the
  /// FILES app shows for the same file.
  ///
  /// This was a local binary ladder, and its own comment gave the goal it was
  /// missing: "that is what a file manager shows and the viewer should not
  /// disagree with them". It was written against `ls -lh` rather than against the
  /// file manager in this system, which uses the kit's 1000-based ladder, so the
  /// two disagreed about the same file - 84213 bytes read `82.2 KiB` here and
  /// `84 KB` there. A person checking a size in two windows got two answers.
  ///
  /// It also called `toFixed(1)`, which writes a period decimal in every
  /// language, so a German reader saw `1.5 MiB` where the rest of their machine
  /// says `1,5`. The kit hands the number to `Intl` with the app's locale, so
  /// that comes free with the agreement.

  function readableDuration(seconds: number): string {
    const s = Math.round(seconds);
    const m = Math.floor(s / 60);
    return `${m}:${String(s % 60).padStart(2, "0")}`;
  }

  function channelsLabel(n: number): string {
    if (n === 1) return $t("v.channelsMono");
    if (n === 2) return $t("v.channelsStereo");
    return $t("v.channelsN", { count: n });
  }

  /// The file the arrow keys move from. Held because the neighbour lookup needs a
  /// path, and `loaded` carries only what the surface renders (a name, a title) -
  /// which is not enough to find what is beside it on disk.
  let currentPath: string | null = $state(null);

  /// Load one file into the viewer. Split out of `onMount` so a keypress can do
  /// what the launch does: the two paths must not drift, or arrowing to a picture
  /// would show it differently from opening it directly.
  async function openFile(path: string) {
    quarters = 0;
    const name = basename(path);
    // Asked for, not assumed. `null` when the folder cannot be read or the file
    // is not in it, and the surface then shows no position rather than a made-up
    // one - the whole reason this call exists.
    let at: [number, number] | null = null;
    try {
      at = await invoke<[number, number] | null>("folder_position", { path });
    } catch {
      // A folder that will not list is not a reason to refuse to show the file.
    }
    try {
      const kind = await invoke<string>("detect_media_kind", { path });
      if (kind === "image") {
        const raster = await invoke<Raster>("decode_image", { path });
        facts = [
          { label: $t("v.factName"), value: name },
          { label: $t("v.factKind"), value: kind },
          { label: $t("v.factDimensions"), value: `${raster.width} × ${raster.height}` },
          ...(await statFacts(path)),
        ];
        loaded = { kind: "image", file: { name, index: at?.[0], total: at?.[1] }, raster };
      } else if (kind === "audio") {
        const info = await invoke<AudioInfo>("probe_audio", { path });
        facts = [
          { label: $t("v.factName"), value: name },
          { label: $t("v.factKind"), value: kind },
          { label: $t("v.factTitle"), value: info.title },
          { label: $t("v.factArtist"), value: info.artist },
          { label: $t("v.factCodec"), value: info.codec },
          { label: $t("v.factSampleRate"), value: `${info.sample_rate} Hz` },
          { label: $t("v.factChannels"), value: channelsLabel(info.channels) },
          {
            label: $t("v.factDuration"),
            // Absent rather than "0:00" when the container declares no length:
            // a duration the file does not state is not a duration of zero.
            value: info.duration_ms === null ? null : readableDuration(info.duration_ms / 1000),
          },
          ...(await statFacts(path)),
        ];
        loaded = {
          kind: "audio",
          file: {
            // Real tags from the probe, falling back to the file name for an
            // untagged file.
            title: info.title ?? name,
            artist: info.artist,
            codec: info.codec,
            durationSec: (info.duration_ms ?? 0) / 1000,
            // The real waveform from the probe's decode pass, scaled from the
            // probe's 0-255 bytes into the 0..1 the Waveform documents. Without
            // the divide every sample was >= 1, the silhouette clipped to full
            // height, and a 17-second speech recording drew as one solid block -
            // while the demo looked perfect, because `mockPeaks` was already
            // 0..1. A surface that had only ever been seen with fixture data.
            // No peaks means NO peaks. The fallback here used to be `mockPeaks()`,
            // which drew an invented silhouette for a real file whose decode pass
            // returned nothing - and a waveform is not decoration, it is a claim
            // about what is in the audio, made on the same surface that shows the
            // file's own name. The demo faces above are acknowledged fixtures
            // reachable with no host or an explicit `?demo=`; this is the real path
            // with a real file, where a sample is not the answer.
            //
            // `Waveform` returns early on an empty array, so this draws nothing at
            // all - not a flat line, which would be its own false claim, that the
            // file is silent. The transport, the times and the seek target stay.
            peaks: info.peaks.length ? info.peaks.map((p) => p / 255) : [],
            index: at?.[0],
            total: at?.[1],
          },
        };
      } else {
        loadError = `unsupported media kind: ${kind}`;
      }
    } catch (e) {
      loadError = String(e);
    }
  }

  /// Size and modification time, or nothing at all. A file that cannot be
  /// stat-ed still opens and still shows everything else; the two rows are simply
  /// not there.
  async function statFacts(path: string): Promise<Fact[]> {
    try {
      const f = await invoke<{ size_bytes: number; modified_ms: number | null }>("file_facts", {
        path,
      });
      return [
        { label: $t("v.factSize"), value: formatSize(f.size_bytes, $locale) },
        {
          // The app's locale, not the environment's. A bare `toLocaleString()`
          // asks the browser, which on a machine set to English shows an English
          // date under a German surface - the one fact on this panel that is not
          // a number, written in a language the rest of the window is not.
          label: $t("v.factModified"),
          value:
            f.modified_ms === null
              ? null
              : new Date(f.modified_ms).toLocaleString($locale),
        },
      ];
    } catch {
      return [];
    }
  }

  /// The last delete, kept until it is undone or superseded.
  ///
  /// Everything an undo needs is in here because `trash_file` returned it - the
  /// app holds no hidden state about what it deleted, so the offer to undo cannot
  /// outlive the knowledge of what to undo.
  type Deleted = { trashed: string; info: string; original: string; name: string };
  let lastDeleted = $state<Deleted | null>(null);

  /// Quarter turns the picture is shown at, reset whenever a different file is
  /// loaded: rotating one photo must not silently rotate the next.
  let quarters = $state(0);

  /// A failed ACTION, as opposed to a file that would not open.
  ///
  /// These are different states and putting them in one variable hid a real one:
  /// `loadError` is only rendered when nothing is loaded (the branch for a picture
  /// wins over it), so a delete that failed while a picture was on screen set a
  /// message nobody could see - the file stayed, the app said nothing, and the
  /// keypress looked ignored. Measured on 16 August with a file on a tmpfs, where
  /// the trash move fails with EXDEV.
  let actionError = $state<string | null>(null);

  /// Delete the open file to the trash and move on to the next one.
  ///
  /// TRASH, NEVER UNLINK. The delete key in a viewer is pressed by someone looking
  /// at a picture, usually while moving quickly, and the only acceptable answer to
  /// "that was the wrong one" is that it comes back.
  ///
  /// The neighbour is resolved BEFORE the delete, because afterwards the file is
  /// gone from the folder and has no neighbours - asking then would answer about a
  /// file that no longer exists.
  async function deleteCurrent() {
    if (!tauriAvailable || !currentPath) return;
    actionError = null;
    const doomed = currentPath;
    const name = basename(doomed);
    let neighbour: string | null = null;
    try {
      neighbour = await invoke<string | null>("neighbour_file", {
        path: doomed,
        direction: "next",
      });
    } catch {
      // No neighbour is a fine answer; the delete still happens.
    }
    try {
      const t = await invoke<{ trashed: string; info: string; original: string }>("trash_file", {
        path: doomed,
      });
      lastDeleted = { ...t, name };
    } catch (e) {
      // The host names WHICH refusal; the sentence is written here so it is in
      // the reader's language. An answer this does not model is shown as it came.
      const p = trashProblem(String(e));
      actionError =
        p.key === "v.couldNotDelete"
          ? $t("v.couldNotDelete", { reason: p.detail })
          : p.key === "v.trash.noTrashHere"
            ? $t("v.trash.noTrashHere")
            : p.key === "v.trash.io"
              ? $t("v.trash.io", { message: p.detail })
              : p.key === "v.trash.crossDevice"
                ? $t("v.trash.crossDevice")
                : p.key === "v.trash.notFound"
                  ? $t("v.trash.notFound")
                  : p.key === "v.trash.unsupported"
                    ? $t("v.trash.unsupported")
                    : p.key === "v.trash.noSlot"
                      ? $t("v.trash.noSlot")
                      : $t("v.trash.nonCanonical");
      return;
    }
    if (neighbour && neighbour !== doomed) {
      currentPath = neighbour;
      await openFile(neighbour);
    } else {
      // It was the only file its kind in the folder: there is nothing to show, and
      // saying so is better than leaving the deleted picture on screen.
      loaded = null;
      currentPath = null;
      noFile = true;
    }
  }

  /// What the print portal last said, so the person is told rather than left
  /// guessing whether anything happened.
  let printStatus = $state<string | null>(null);

  /// Hand the open picture to the print portal.
  ///
  /// The portal, not a printer: this app has no idea what printers exist, and
  /// should not - it hands over a file descriptor and the portal takes it from
  /// there. `print_file` waits for the answer, so the pending state below is the
  /// dialog actually being open rather than a guess.
  async function printCurrent() {
    if (!tauriAvailable || !currentPath) return;
    const name = currentPath.split("/").pop() ?? currentPath;
    printStatus = $t("v.printing");
    try {
      const r = await invoke<{ outcome: string }>("plugin:arlen-shell|print_file", { path: currentPath });
      printStatus =
        r.outcome === "sent"
          ? $t("v.printSent", { name })
          : r.outcome === "cancelled"
            ? $t("v.printCancelled")
            : r.outcome === "refused"
              ? $t("v.printRefused")
              : $t("v.printNoAnswer");
    } catch (e) {
      printStatus = null;
      const p = printProblem(String(e));
      actionError =
        p.key === "v.print.noPortal"
          ? $t("v.print.noPortal")
          : p.key === "v.print.noBus"
            ? $t("v.print.noBus")
            : p.key === "v.print.fileUnreadable"
              ? $t("v.print.fileUnreadable", { message: p.detail })
              : $t("v.couldNotPrint", { reason: p.detail });
    }
  }

  /// Put the last deleted file back and show it again.
  async function undoDelete() {
    if (!tauriAvailable || !lastDeleted) return;
    const d = lastDeleted;
    try {
      await invoke("restore_file", { trashed: d.trashed, info: d.info, original: d.original });
    } catch (e) {
      // Same shape as the delete: the host names which refusal, the window
      // writes it. "Something is using that name again" is the one that happens.
      const p = restoreProblem(String(e));
      actionError =
        p.key === "v.restore.nameTaken"
          ? $t("v.restore.nameTaken")
          : p.key === "v.restore.unsupported"
            ? $t("v.restore.unsupported")
            : p.key === "v.restore.crossDevice"
              ? $t("v.restore.crossDevice")
              : $t("v.couldNotRestore", { reason: p.detail });
      return;
    }
    lastDeleted = null;
    noFile = false;
    loadError = null;
    actionError = null;
    currentPath = d.original;
    await openFile(d.original);
  }

  /// Arrow keys walk the folder, which is the behaviour that makes this a viewer
  /// rather than a file-opener - opening one picture puts you in the folder, the
  /// way imv does.
  ///
  /// A `null` answer means there is nowhere to go: a lone picture, or a file the
  /// viewer cannot show. The view is left exactly as it is, because moving to
  /// nothing and blanking the surface would read as the file having failed.
  async function step(direction: "next" | "previous") {
    if (!tauriAvailable || !currentPath) return;
    try {
      const neighbour = await invoke<string | null>("neighbour_file", {
        path: currentPath,
        direction,
      });
      if (!neighbour) return;
      currentPath = neighbour;
      await openFile(neighbour);
    } catch (e) {
      loadError = String(e);
    }
  }

  function onKey(event: KeyboardEvent) {
    // Ctrl+Z undoes the last delete, which is the one modifier combination this
    // window claims - and it is claimed before the modifier guard below, which
    // exists to keep every OTHER combination behaving as it does everywhere else.
    if (event.ctrlKey && !event.altKey && !event.metaKey && event.key.toLowerCase() === "z") {
      event.preventDefault();
      void undoDelete();
      return;
    }
    // Ignored with a modifier held: Ctrl+Right is a word-jump everywhere else,
    // and a viewer that swallowed it would be the one application that does not
    // behave.
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.key === "Delete") {
      event.preventDefault();
      void deleteCurrent();
      return;
    }
    if (event.key === "ArrowRight" || event.key === "ArrowDown" || event.key === " ") {
      event.preventDefault();
      void step("next");
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      event.preventDefault();
      void step("previous");
    } else if (event.key === "r" || event.key === "R") {
      // R turns clockwise, Shift+R the other way - one key for the common case and
      // the same key reversed for the other, rather than two keys to remember.
      event.preventDefault();
      quarters = (((quarters + (event.shiftKey ? -1 : 1)) % 4) + 4) % 4;
    } else if (event.key === "i" || event.key === "I") {
      // Toggles, so the key that opens it closes it - the plan gives `I` and no
      // second key, and a panel with no way back is a trap.
      event.preventDefault();
      detailsOpen = !detailsOpen;
    } else if (event.key === "Escape") {
      detailsOpen = false;
    }
  }
</script>

<svelte:window on:keydown={onKey} />

{#snippet face(d: string)}
  {#if d === "audio"}
    <AudioPlayer file={audioMock} />
  {:else if d === "image"}
    <ImageViewer file={imageMock} />
  {:else if d === "video"}
    <VideoViewer file={videoMock} />
  {/if}
{/snippet}

{#snippet appTitle()}
  <!-- The page's one level-one heading. Every app in this tree had none, so a
       screen reader's first question - what IS this window - was answered only
       by the window title, which is not in the document. A snippet rather than
       six copies: the branches are mutually exclusive, so exactly one renders,
       and it lives inside the branch's `main` so it is inside the landmark. -->
  <h1 class="sr-only">{$t("v.app.title")}</h1>
{/snippet}

<!-- Every branch is the whole page, and they are mutually exclusive - so each is
     `main`, and exactly one main renders. A viewer with no `main` leaves a
     screen-reader user nothing to skip to; the failure states are content too. -->
{#if loaded?.kind === "image"}
  <!-- The chevrons and the arrow keys are the same move. They were drawn but
       unwired, so a viewer that looked navigable did nothing when clicked. -->
  <main class="fill">
    {@render appTitle()}
    <ImageViewer
      file={loaded.file}
      raster={loaded.raster}
      {quarters}
      onnext={() => step("next")}
      onprev={() => step("previous")}
      onprint={tauriAvailable ? printCurrent : undefined}
    />
    {#if detailsOpen}
      <DetailsPanel {facts} onclose={() => (detailsOpen = false)} />
    {/if}
  </main>
{:else if loaded?.kind === "audio"}
  <main class="fill">
    {@render appTitle()}
    <AudioPlayer file={loaded.file} onnext={() => step("next")} onprev={() => step("previous")} />
    {#if detailsOpen}
      <DetailsPanel {facts} onclose={() => (detailsOpen = false)} />
    {/if}
  </main>
{:else if loadError}
  <!-- With the window controls, which this branch did without until 16 August.
       The window is frameless, so the close button lives inside whichever view is
       rendered - and these two branches render no view. A file that failed to open
       therefore left a bare sentence in a window with nothing to close it, which is
       the one state where a person most wants that button. Found by opening the
       viewer on a file it cannot read and looking at the result. -->
  <main class="fill err">
    {@render appTitle()}
    <div class="winctl">
      <WindowButtons showMaximize={false} />
    </div>
    <!-- Announced: this replaces the whole view after the person opened a file,
         so somebody who cannot see it has no other signal that the open did not
         take - the window simply stops showing anything. -->
    <p role="alert">
      {readsAsInternal(loadError)
        ? $t("v.couldNotOpenUnknown")
        : $t("v.couldNotOpen", { reason: loadError })}
    </p>
  </main>
{:else if noFile}
  <!-- Before the demo branches on purpose: in the real shell an empty window is
       an empty window, and the sample below is for the harness and the browser. -->
  <main class="fill err">
    {@render appTitle()}
    <div class="winctl">
      <WindowButtons showMaximize={false} />
    </div>
    {$t("v.nothingOpen")}
  </main>
{:else if framed}
  <main class="frame" style="width:{w}px;height:{h}px">
    {@render appTitle()}
    {@render face(demo)}
  </main>
{:else}
  <main class="fill">
    {@render appTitle()}
    {@render face(demo)}
  </main>
{/if}

{#if printStatus}
  <!-- Its own bar rather than the error one: a print that was cancelled, or a
       dialog still waiting, is not a failure and must not be dressed as one. -->
  <div class="undobar" role="status">
    <span>{printStatus}</span>
    <button onclick={() => (printStatus = null)}>{$t("v.close")}</button>
  </div>
{:else if actionError}
  <!-- Over whatever is on screen, because that is where the failure happened. The
       load-error branch below cannot serve here: it only renders when nothing is
       loaded, so it is invisible in exactly the case an action fails. -->
  <div class="undobar bad" role="alert">
    <span>{actionError}</span>
    <button onclick={() => (actionError = null)}>{$t("v.close")}</button>
  </div>
{:else if lastDeleted}
  <!-- Over every state, including "no file is open" - that is precisely the state
       left behind by deleting the last picture in a folder, and the moment the
       offer matters most. -->
  <div class="undobar" role="status">
    <span>{$t("v.movedToTrash", { name: lastDeleted.name })}</span>
    <button onclick={() => undoDelete()}>{$t("v.undo")}</button>
  </div>
{/if}

<style>
  :global(body) {
    margin: 0;
  }
  .undobar {
    position: fixed;
    left: 50%;
    bottom: 68px;
    transform: translateX(-50%);
    z-index: 30;
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 10px 8px 14px;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, #141414 92%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-primary, #fafafa) 14%, transparent);
    color: var(--color-fg-primary, #fafafa);
    font-size: 13px;
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(12px);
  }
  .undobar button {
    all: unset;
    padding: 4px 10px;
    border-radius: 8px;
    font-weight: 600;
    background: color-mix(in srgb, var(--color-fg-primary, #fafafa) 14%, transparent);
  }
  .undobar.bad {
    border-color: color-mix(in srgb, var(--color-error, #ef4444) 55%, transparent);
  }
  .undobar button:hover {
    background: color-mix(in srgb, var(--color-fg-primary, #fafafa) 22%, transparent);
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
    /* The details panel positions against the window, so this is its root. */
    position: relative;
  }
  /* Top-right, over the message. No auto-hide here, unlike the viewers: there is
     no picture underneath for the chrome to be in the way of, and a control that
     appears only on mouse movement is a poor answer to "how do I close this". */
  .err .winctl {
    position: absolute;
    top: 8px;
    right: 8px;
  }

  .err {
    position: relative;
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
