<script lang="ts">
  /// App shell: locale and theme come from the system, like every other app.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { t } from "$lib/i18n/messages";
  import { setWindowTitle } from "$lib/window-title";
  import { unreadCount } from "$lib/stores/mailbox";
  import { publishBadge } from "$lib/badge";

  let { children } = $props();

  /// The top bar's badge, from the same count the folder rail draws.
  ///
  /// Here rather than on the page: a badge is per APP, not per view
  /// (`badges-api.md` FA1/FA4), and the shell keeps one entry per app id whether
  /// this window is focused or not. Republished on every change because the
  /// shell holds the value and nothing recomputes it - the same reason the menu
  /// re-registers when its labels or its state move.
  $effect(() => {
    void publishBadge($unreadCount);
  });

  // The topbar and the workspace overview show the NATIVE window title,
  // not the document one below, so it has to be set - and set again when
  // the language changes, which is why this reads `$t` instead of firing
  // once at startup.
  $effect(() => {
    void setWindowTitle($t("ml.app.title"));
  });

  onMount(() => {
    void initArlenLocale();
    void initArlenTheme();
  });
</script>

<svelte:head>
  <title>{$t("ml.app.title")}</title>
</svelte:head>

{@render children()}
