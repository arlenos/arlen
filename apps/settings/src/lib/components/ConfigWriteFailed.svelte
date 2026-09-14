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
  import { Notice } from "@arlen/ui-kit/components/ui/notice";

  let { failed }: { failed: boolean } = $props();
</script>

<!-- The kit Notice, in the one shape every refusal has (design-system.md 6.11,
     thread two). It sits inside the page's SectionGrid, spanning it, so it gets
     the grid's column rather than sitting 120px left of everything else. -->
{#if failed}
  <Notice tone="error" class="span-full" text={$t("s.config.notSaved")} />
{/if}
