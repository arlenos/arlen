<script lang="ts">
  /// The unified consent dialog (system-dialog-plan.md): one polymorphic surface
  /// every permission request routes into, the sibling of the App-access page.
  /// Mounted once in +layout, inert when nothing is pending. The frame is common
  /// to every request (attested requester, risk/outcome summary, scope); the body
  /// and the affordances vary by severity tier and class - a standard grant offers
  /// deny / once / remember, a high-stakes destructive demands a hold-to-confirm
  /// and never remembers, an external send names the recipient. Deny is always
  /// first-class; the least-privilege default is "once".
  import { onMount } from "svelte";
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { AlertTriangle, Send, Trash2 } from "lucide-svelte";
  import { current, resolve, pollConsent, type PendingView } from "$lib/stores/consent";

  onMount(() => {
    void pollConsent();
  });

  const NAMES: Record<string, string> = {
    "org.arlen.files": "Files",
    "org.arlen.installd": "Software install",
    "com.example.notes": "Notes",
    "com.example.mail": "Mail",
  };
  function friendly(id: string): string {
    const seg = id.split(".").pop() ?? id;
    return NAMES[id] ?? seg.charAt(0).toUpperCase() + seg.slice(1);
  }

  function deny(p: PendingView) {
    void resolve(p.id, "denied");
  }
  function allowOnce(p: PendingView) {
    void resolve(p.id, "allowed_once");
  }
  function allowRemember(p: PendingView) {
    void resolve(p.id, "allowed_remembered");
  }

  // Hold-to-confirm for the destructive class: a press-and-hold fills the button
  // over ~1.2s, then fires. Releasing early cancels. The confirm delay is the
  // anti-accident affordance for the one class that cannot be undone.
  let holding = $state(false);
  let holdTimer: ReturnType<typeof setTimeout> | null = null;
  function holdStart(p: PendingView) {
    holding = true;
    holdTimer = setTimeout(() => {
      holding = false;
      allowOnce(p);
    }, 1200);
  }
  function holdEnd() {
    holding = false;
    if (holdTimer) clearTimeout(holdTimer);
    holdTimer = null;
  }

</script>

{#if $current}
  {@const p = $current}
  <!-- The gate is reversibility, not impact (system-dialog-plan.md): reversible
       actions get the generous remember (it carries autonomous authority);
       only the genuinely irreversible confirm per instance. Destructive is NOT
       automatically irreversible - move-to-Trash is reversible. -->
  {@const holdDestructive = p.class === "destructive" && p.reversibility === "irreversible"}
  {@const standingElsewhere = p.class === "external_send" || p.class === "elevated_privilege"}
  {@const irreversibleOther = p.reversibility === "irreversible" && !holdDestructive && !standingElsewhere}
  {@const reversibleDestructive = p.class === "destructive" && p.reversibility !== "irreversible"}
  <Dialog.Root
    open={true}
    onOpenChange={(open) => {
      if (!open) deny(p);
    }}
  >
    <Dialog.Content>
      <div class="cd">
        <div class="cd-req">
          <span class="cd-avatar">{friendly(p.requester).charAt(0)}</span>
          <span class="cd-req-name">{friendly(p.requester)}</span>
          <span class="cd-req-id">{p.requester}</span>
        </div>

        <h2 class="cd-title">Allow {friendly(p.requester)} to {p.summary}?</h2>

        {#if p.scope}
          <p class="cd-scope">
            {#if p.class === "external_send"}To{:else if p.class === "destructive"}Target{:else}Scope{/if}
            <span class="cd-scope-val">{p.scope}</span>
          </p>
        {/if}

        {#if holdDestructive || irreversibleOther}
          <div class="cd-warn danger">
            <AlertTriangle size={14} strokeWidth={2} />
            This cannot be undone.
          </div>
        {:else if standingElsewhere}
          <div class="cd-warn">
            <AlertTriangle size={14} strokeWidth={2} />
            This acts outside Arlen. Only continue if you started it.
          </div>
          <p class="cd-note">
            To let {friendly(p.requester)} do this on its own, allow it in App access.
          </p>
        {:else if reversibleDestructive}
          <p class="cd-note">You can undo this from the Trash.</p>
        {/if}

        <div class="cd-foot">
          {#if holdDestructive}
            <Button variant="outline" onclick={() => deny(p)}>Cancel</Button>
            <span class="cd-spacer"></span>
            <button
              type="button"
              class="cd-hold"
              class:holding
              onpointerdown={() => holdStart(p)}
              onpointerup={holdEnd}
              onpointerleave={holdEnd}
            >
              <span class="cd-hold-fill" aria-hidden="true"></span>
              <span class="cd-hold-label"><Trash2 size={14} strokeWidth={2} /> Hold to delete</span>
            </button>
          {:else if standingElsewhere}
            <Button variant="outline" onclick={() => deny(p)}>Deny</Button>
            <span class="cd-spacer"></span>
            {#if p.class === "external_send"}
              <Button onclick={() => allowOnce(p)}><Send size={14} strokeWidth={2} /> Send once</Button>
            {:else}
              <Button onclick={() => allowOnce(p)}>Allow once</Button>
            {/if}
          {:else if irreversibleOther}
            <Button variant="outline" onclick={() => deny(p)}>Deny</Button>
            <span class="cd-spacer"></span>
            <Button onclick={() => allowOnce(p)}>Allow once</Button>
          {:else}
            <Button variant="outline" onclick={() => deny(p)}>Deny</Button>
            <span class="cd-spacer"></span>
            <Button variant="ghost" onclick={() => allowRemember(p)}>Always allow</Button>
            <Button onclick={() => allowOnce(p)}>Allow once</Button>
          {/if}
        </div>
      </div>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .cd {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
  }
  .cd-req {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.75rem;
  }
  .cd-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .cd-req-name {
    font-weight: 600;
    color: var(--foreground);
  }
  .cd-req-id {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .cd-title {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    line-height: 1.35;
    color: var(--foreground);
  }
  .cd-scope {
    margin: 0;
    display: flex;
    gap: 0.4rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .cd-scope-val {
    font-family: var(--font-mono, ui-monospace, monospace);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cd-warn {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.625rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .cd-warn.danger {
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    color: var(--color-error);
  }
  /* A quiet reassurance / pointer (reversible undo, or where standing access
     lives) - not a wall, just a line. */
  .cd-note {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  .cd-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-top: 0.375rem;
  }
  .cd-spacer {
    flex: 1;
  }

  /* The destructive hold-to-confirm: a filled bar sweeps left-to-right over the
     hold, the label rides on top. Error-toned, its own control (not a Button). */
  .cd-hold {
    position: relative;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: var(--height-control, 32px);
    padding: 0 0.75rem;
    border: 1px solid color-mix(in srgb, var(--color-error) 45%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-error) 10%, transparent);
    color: var(--color-error);
    font-size: 0.8125rem;
    font-weight: 500;
    cursor: pointer;
    user-select: none;
  }
  .cd-hold-fill {
    position: absolute;
    inset: 0;
    width: 0;
    background: color-mix(in srgb, var(--color-error) 30%, transparent);
  }
  .cd-hold.holding .cd-hold-fill {
    width: 100%;
    transition: width 1.2s linear;
  }
  .cd-hold-label {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
  }
</style>
