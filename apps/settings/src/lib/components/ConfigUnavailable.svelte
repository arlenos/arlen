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
  /* The same centred column `Page` gives its header and `SectionGrid` gives the
     cards. A child dropped straight into `Page` gets neither, so this line sat
     against the page padding while every other thing on the page began 120px
     further right - measured at 1280px: heading and section labels at x=401,
     this at x=281, hard against the sidebar.

     Invisible until this week, because the screenshot harness rendered at 640px
     where the column is the whole width and nothing can be out of it. */
  .config-unavailable {
    width: 100%;
    max-width: var(--width-section-body, 46rem);
    margin: 0 auto 0.75rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--color-error, #f87171);
  }
</style>
