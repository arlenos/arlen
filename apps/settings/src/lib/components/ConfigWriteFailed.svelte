<script lang="ts">
  /// The line a settings page shows when a change it made was refused.
  ///
  /// The sibling of `ConfigUnavailable`, and the two must stay apart because they
  /// are different facts with different things to do about them: one says the
  /// values on screen were never read, the other says the value on screen is the
  /// one that is stored, and the change you just made is not.
  ///
  /// Until 6 September there was nothing here at all. `setValue` put the reason
  /// in the store's `error` and then called `load()` to roll back, and `load()`
  /// clears `error` on its first line - so the failure was erased by the recovery
  /// and a refused write showed a control sliding back on its own, silently, on
  /// every page in this app.
  ///
  /// ONE CLAUSE (design-system §6.7). The rollback re-reads disk, so the control
  /// already shows what the system holds; the second clause that case sometimes
  /// wants - "the control shows what you set, the system still has the old value"
  /// - would be false here, because the control does not.
  import { t } from "$lib/i18n/messages";

  let { failed }: { failed: boolean } = $props();
</script>

{#if failed}
  <p class="config-write-failed" role="alert">{$t("s.config.notSaved")}</p>
{/if}

<style>
  /* The same centred column as `ConfigUnavailable`, for the same reason recorded
     there: a child dropped straight into `Page` gets neither the header's column
     nor the grid's, and sits 120px left of everything else on the page. */
  .config-write-failed {
    width: 100%;
    max-width: var(--width-section-body, 46rem);
    margin: 0 auto 0.75rem;
    font-size: 0.85rem;
    font-weight: 500;
    color: var(--color-error, #f87171);
  }
</style>
