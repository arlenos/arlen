<script lang="ts">
  /// Compose: To, Subject, the body, Send. Sending has no account behind it
  /// yet, and the surface says so in one sentence instead of pretending - the
  /// press files the draft into Drafts, which is the true thing it can do.
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { t } from "$lib/i18n/messages";
  import { saveDraft } from "$lib/stores/mailbox";

  let {
    presetTo = "",
    presetSubject = "",
    presetBody = "",
    ondone,
  }: {
    presetTo?: string;
    presetSubject?: string;
    presetBody?: string;
    ondone: (draftId: string | null) => void;
  } = $props();

  // The presets seed the fields once; the page keys this component on the
  // preset object, so a new Reply/Forward mounts fresh rather than mutating a
  // half-written draft under the writer.
  // svelte-ignore state_referenced_locally
  let to = $state(presetTo);
  // svelte-ignore state_referenced_locally
  let subject = $state(presetSubject);
  // svelte-ignore state_referenced_locally
  let body = $state(presetBody);

  // NAMED FOR WHAT IT DOES. This was `send`, behind a button reading Send, and it
  // has only ever written a draft - there is no account to send through and no
  // submission path behind it. The standing note under the fields explained that,
  // which is not the same as the control being honest: a reader presses the verb,
  // not the paragraph. When sending exists, the label and this function change
  // together, and nothing else has to.
  function saveToDrafts(): void {
    const id = saveDraft(to, subject, body);
    ondone(id);
  }
</script>

<div class="compose">
  <div class="fields">
    <label class="field">
      <span class="k">{$t("ml.compose.to")}</span>
      <Input id="compose-to" bind:value={to} />
    </label>
    <label class="field">
      <span class="k">{$t("ml.compose.subject")}</span>
      <Input id="compose-subject" bind:value={subject} />
    </label>
  </div>
  <Textarea
    id="compose-body"
    class="compose-body"
    bind:value={body}
    rows={10}
    maxRows={24}
    placeholder={$t("ml.compose.body")}
    aria-label={$t("ml.compose.body")}
  />
  <p class="cant-send">{$t("ml.compose.cantSend")}</p>
  <div class="actions">
    <Button id="compose-save-draft" onclick={saveToDrafts}>{$t("ml.compose.saveToDrafts")}</Button>
    <Button variant="ghost" id="compose-discard" onclick={() => ondone(null)}>{$t("ml.compose.discard")}</Button>
  </div>
</div>

<style>
  .compose {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    width: 100%;
    max-width: 46rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
    flex: 1;
    min-height: 0;
  }
  .fields {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .field {
    display: grid;
    grid-template-columns: 5rem 1fr;
    align-items: center;
    gap: 0.6rem;
  }
  .k {
    font-size: var(--text-sm, 13px);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .cant-send {
    margin: 0;
    font-size: var(--text-xs, 12px);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .actions {
    display: flex;
    gap: 0.5rem;
  }
</style>
