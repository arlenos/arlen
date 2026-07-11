<script lang="ts">
  /// The AI action-scope authorization prompt, rendered through the shared
  /// `ConsentCard` so it speaks the same visual language as the rest of the
  /// permission cluster (system-dialog-plan.md). The AI daemon asks to use an
  /// MCP action scope; the user allows it for this session or denies.
  ///
  /// Mounted once globally in `+layout.svelte`. Inert when no prompt is active.
  /// Closing any way other than Allow is a denial (Escape, backdrop, Deny).
  ///
  /// Footer: today only Deny / Allow-once — the transport
  /// (`ai_respond_authorization`, boolean) cannot persist a grant, so the
  /// reversibility-gated "Always allow" for the reversible scopes (calendar,
  /// file-management, per the 11-Jul ruling) waits on a remember seam (a
  /// `remember` flag + a revocable Grant node), flagged to the coder.
  import { onMount, onDestroy } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import ConsentCard from "$lib/components/ConsentCard.svelte";
  import { current, init, dispose, respond } from "$lib/stores/aiAuthorization";

  onMount(() => {
    init();
  });

  onDestroy(() => {
    dispose();
  });

  /// Phrase a scope as a plain-language capability. Unknown scopes fall back to
  /// the raw scope string rather than guessing.
  function scopeLabel(scope: string): string {
    const known: Record<string, string> = {
      "file-management": "create, move and delete files",
      terminal: "run shell commands",
      calendar: "create and modify calendar events",
      email: "compose and send email",
    };
    return known[scope] ?? `use "${scope}"`;
  }

  /// Reaching outside the assistant's own workspace (run a shell, send mail) is
  /// caution-toned; a plain capability grant stays neutral.
  function toneOf(scope: string): "neutral" | "caution" {
    return scope === "terminal" || scope === "email" ? "caution" : "neutral";
  }
</script>

{#if $current}
  {@const prompt = $current}

  {#snippet body()}
    <p class="aa-note">
      This lasts for the current session only. It is not remembered, and it
      covers nothing beyond this request.
    </p>
  {/snippet}

  {#snippet footer()}
    <Button variant="outline" onclick={() => respond(prompt.promptId, false)}>Deny</Button>
    <span style="flex:1"></span>
    <Button onclick={() => respond(prompt.promptId, true)}>Allow once</Button>
  {/snippet}

  <Dialog.Root
    open={true}
    onOpenChange={(open) => {
      if (!open) respond(prompt.promptId, false);
    }}
  >
    <Dialog.Content>
      <ConsentCard
        requesterName="Assistant"
        requesterId="org.arlen.ai"
        tone={toneOf(prompt.scope)}
        title={`Allow the assistant to ${scopeLabel(prompt.scope)}?`}
        {body}
        {footer}
      />
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .aa-note {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
