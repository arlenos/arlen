<script lang="ts">
  /// A reusable screen-reader live region: a visually hidden element whose text is
  /// announced when it changes. Mount it once per app and update `message` to speak
  /// (a view change, a stream update, a status). Nothing is shown; this is behaviour,
  /// not chrome.
  let {
    message = "",
    assertive = false,
  }: {
    /// The text to announce. Changing it triggers the announcement.
    message?: string;
    /// Interrupt the user (`assertive`) instead of waiting for a pause (`polite`).
    assertive?: boolean;
  } = $props();
</script>

<div class="sr-only" role="status" aria-live={assertive ? "assertive" : "polite"} aria-atomic="true">
  {message}
</div>

<style>
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
</style>
