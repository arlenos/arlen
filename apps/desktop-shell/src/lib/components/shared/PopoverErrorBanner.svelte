<script lang="ts">
  /// Error strip for a popover body — the tinted one-liner Network
  /// and Bluetooth used to declare identically. Sits at the top of
  /// the body, above the content it talks about.
  let { message }: { message: string } = $props();
</script>

<div class="pop-error">{message}</div>

<style>
  .pop-error {
    padding: 6px 10px;
    background: color-mix(in srgb, var(--color-error) 15%, transparent);
    border-radius: var(--radius-input);
    color: var(--color-error);
    font-size: var(--text-2xs);
  }
</style>
