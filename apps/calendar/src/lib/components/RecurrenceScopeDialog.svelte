<script lang="ts">
  /// The three-way series question - the dialog cheap calendars skip and get
  /// remembered for skipping. Asked before an edit, a move or a delete of a
  /// repeating occurrence; the middle answer ("this and following") is the one
  /// whose absence is a named grievance elsewhere.
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t } from "$lib/i18n/messages";

  export type Scope = "this" | "following" | "all";

  let {
    open,
    action,
    onpick,
    oncancel,
  }: {
    open: boolean;
    action: "edit" | "delete" | "move";
    onpick: (scope: Scope) => void;
    oncancel: () => void;
  } = $props();

  const question = $derived(
    action === "delete" ? $t("cal.scope.deleteQ") : action === "move" ? $t("cal.scope.moveQ") : $t("cal.scope.editQ"),
  );
</script>

<Dialog {open} onClose={oncancel} size="sm" ariaLabel={$t("cal.scope.title")}>
  <div class="scope">
    <h2 class="s-title">{$t("cal.scope.title")}</h2>
    <p class="s-q">{question}</p>
    <div class="s-choices">
      <Button variant="outline" id="scope-this" onclick={() => onpick("this")}>{$t("cal.scope.this")}</Button>
      <Button variant="outline" id="scope-following" onclick={() => onpick("following")}>
        {$t("cal.scope.following")}
      </Button>
      <Button variant="outline" id="scope-all" onclick={() => onpick("all")}>{$t("cal.scope.all")}</Button>
    </div>
    <div class="s-foot">
      <Button variant="ghost" id="scope-cancel" onclick={oncancel}>{$t("cal.form.cancel")}</Button>
    </div>
  </div>
</Dialog>

<style>
  /* The clock dialog's inset, the house register for a modal's inside. */
  .scope {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1.1rem 1.25rem 1rem;
  }
  .s-title {
    margin: 0;
    font-size: var(--text-base, 15px);
    font-weight: 600;
  }
  .s-q {
    margin: 0;
    font-size: var(--text-sm, 13px);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .s-choices {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .s-foot {
    display: flex;
    justify-content: flex-end;
    padding-top: 0.25rem;
  }
</style>
