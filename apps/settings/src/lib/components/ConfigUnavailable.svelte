<script lang="ts">
  /// The line a settings page shows when its config could not be read.
  ///
  /// Every page here is written as `$store.data?.thing ?? DEFAULT`, which is the
  /// right shape for rendering and a silent lie when the read failed: the
  /// controls come up holding the defaults, and nothing on screen distinguishes
  /// "this is how you have it set" from "we could not ask". Measured on 16 August
  /// with no backend, five pages did exactly that - workspaces, accessibility,
  /// system actions, focus and knowledge - while notifications was the only one
  /// rendering its store's `error`.
  ///
  /// It says the values are defaults rather than just naming the failure, because
  /// that is the part a reader cannot see for themselves: a slider at 8 looks
  /// identical whether it was read or invented.
  import { t } from "$lib/i18n/messages";

  let { error }: { error: string | null } = $props();
</script>

{#if error}
  <p class="config-unavailable" role="alert" title={error}>
    {$t("s.config.unavailable")}
  </p>
{/if}

<style>
  .config-unavailable {
    margin: 0 0 0.75rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--color-error, #f87171);
  }
</style>
