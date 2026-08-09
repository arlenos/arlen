<script lang="ts">
  /// Adopt the chosen language and the live theme.
  ///
  /// The locale call has been here since before the app had a host, when the
  /// read inside it could only fail; the host landed on 9 August, so both now
  /// reach the shell plugin the capability file already grants. An app that
  /// takes the theme permission and never reads it is the other half of the
  /// same oversight: the editor would have sat in the default palette while
  /// every other app followed the system.
  import "../app.css";
  import { onMount } from "svelte";
  import { initArlenLocale } from "@arlen/ui-kit/i18n";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  let { children } = $props();

  onMount(() => {
    void initArlenLocale();
    void initArlenTheme();
  });
</script>

{@render children()}
