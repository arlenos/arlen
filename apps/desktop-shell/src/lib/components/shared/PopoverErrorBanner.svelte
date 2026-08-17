<script lang="ts">
  /// Error strip for a popover body — the tinted one-liner Network
  /// and Bluetooth used to declare identically. Sits at the top of
  /// the body, above the content it talks about.
  ///
  /// `role="alert"` because this is the answer to something the person just
  /// pressed. Six panels render this strip and every one of them was silent to
  /// a screen reader: press mute with the audio service refusing, and the strip
  /// appears saying "That change did not reach the audio service." while the
  /// reader hears nothing at all and the switch sits where it was. Each of the
  /// refusal lines written directly into a page carries the role; the shared
  /// primitive, the one used most, was the only one without it.
  ///
  /// Known limit: a live region announces a CHANGE, so this speaks when the
  /// strip appears in an open panel - the case a press produces. A panel that
  /// opens with the failure already known paints the strip in its first frame,
  /// and readers differ on whether they announce an alert that arrives inside a
  /// freshly inserted subtree.
  let { message }: { message: string } = $props();
</script>

<div class="pop-error" role="alert">{message}</div>

<style>
  .pop-error {
    padding: 6px 10px;
    background: color-mix(in srgb, var(--color-error) 15%, transparent);
    border-radius: var(--radius-input);
    color: var(--color-error);
    font-size: var(--text-2xs);
  }
</style>
