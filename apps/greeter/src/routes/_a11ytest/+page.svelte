<script lang="ts">
  /// Headless render harness for the accessibility menu, the `_facettest` pattern.
  ///
  /// The menu carries one sentence that only appears when something went wrong -
  /// the toggle applied but could not be written down - and it cannot be reached
  /// from the running greeter on purpose: it needs the `greeter_a11y_set` command
  /// to reject, which under plain vite it does anyway (there is no Tauri), but
  /// only AFTER somebody clicks the switch, and a screenshot cannot wait for that
  /// reliably.
  ///
  /// `?state=unsaved` puts the menu in the state a person sees on a machine whose
  /// greeter state directory is not writable: the reader is on for this login and
  /// will not be there at the next start. `?locale=de` renders it in German. Not
  /// in any nav; a dev route.
  import { onMount } from "svelte";
  import A11yMenu from "$lib/components/A11yMenu.svelte";
  import { a11y, screenReaderNotRemembered } from "$lib/a11y";
  import { locale } from "@arlen/ui-kit/i18n";

  const params =
    typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  if (params?.get("locale")) locale.set(params.get("locale") as string);
  const unsaved = params?.get("state") === "unsaved";

  let ready = $state(false);

  onMount(() => {
    if (unsaved) {
      // Set directly rather than by tripping the toggle: the toggle's reject path
      // needs a host to reject. The state under test is the sentence, not the
      // path that reaches it.
      a11y.update((s) => ({ ...s, screenReader: true }));
      screenReaderNotRemembered.set(true);
    }
    ready = true;
  });
</script>

<div class="harness">
  {#if ready}
    <A11yMenu />
  {/if}
</div>

<style>
  /* The corner popover opens UPWARD from its trigger, so the trigger has to sit
     low or the panel is clipped off the top of the viewport - which is exactly
     what the first shot of this route showed. */
  .harness {
    min-height: 100vh;
    display: flex;
    align-items: flex-end;
    padding: 2rem;
    background: var(--color-bg-app, #0a0a0a);
  }
</style>
