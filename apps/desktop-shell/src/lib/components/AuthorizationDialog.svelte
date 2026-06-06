<script lang="ts">
  /// Modal that asks the user to authorize an AI action scope.
  ///
  /// Mounted once globally in `+layout.svelte`. Inert when no prompt
  /// is active, so it has zero visual cost in the common case.
  ///
  /// Closing the dialog any way other than Allow counts as a denial:
  /// Escape, backdrop click, and the Deny button all respond with
  /// `false`. The grant covers exactly this scope and lasts only for
  /// the session.

  import { onMount, onDestroy } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@lunaris/ui-kit/components/ui/button";
  import { current, init, dispose, respond } from "$lib/stores/aiAuthorization";

  onMount(() => {
    init();
  });

  onDestroy(() => {
    dispose();
  });

  /// Phrase a scope as a plain-language capability. Unknown scopes
  /// fall back to the raw scope string rather than guessing.
  function scopeLabel(scope: string): string {
    const known: Record<string, string> = {
      "file-management": "create, move and delete files",
      terminal: "run shell commands",
      calendar: "create and modify calendar events",
      email: "compose and send email",
    };
    return known[scope] ?? scope;
  }
</script>

{#if $current}
  {@const prompt = $current}
  <Dialog.Root
    open={true}
    onOpenChange={(open) => {
      if (!open) respond(prompt.promptId, false);
    }}
  >
    <Dialog.Content>
      <Dialog.Header>
        <Dialog.Title>
          Allow the assistant to {scopeLabel(prompt.scope)}?
        </Dialog.Title>
        <Dialog.Description>
          This permission lasts for the current session only. It is not
          remembered, and it covers nothing beyond this request.
        </Dialog.Description>
      </Dialog.Header>
      <Dialog.Footer>
        <Button
          variant="outline"
          onclick={() => respond(prompt.promptId, false)}
        >
          Deny
        </Button>
        <Button onclick={() => respond(prompt.promptId, true)}>Allow</Button>
      </Dialog.Footer>
    </Dialog.Content>
  </Dialog.Root>
{/if}
