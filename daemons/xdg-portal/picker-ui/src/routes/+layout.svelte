<script lang="ts">
  /// Adopt the reader's language before the first dialog is drawn.
  ///
  /// The picker embeds no shell plugin, so `initArlenLocale` falls through to the
  /// bare `locale_get` command the picker's own backend registers. Fails quiet: a
  /// picker that cannot read the choice shows the source language, which is a
  /// working dialog rather than a missing one.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";

  let { children } = $props();

  onMount(() => {
    let unlisten: (() => void) | null = null;
    void initArlenLocale().then((u) => {
      unlisten = u;
    });
    return () => unlisten?.();
  });
</script>

{@render children()}
