<script lang="ts">
  /// The focused profile's authentication, in a flat panel on the wallpaper
  /// (the Waypointer / Quick Settings language): the avatar over the name,
  /// the password field, the hardware-key alternative. Owns the local
  /// attempt state (busy / error / waiting); the parent provides the auth
  /// calls and, on success, greetd takes over. A failed attempt shakes,
  /// clears, and announces over aria-live, never leaving the profile.
  import { onMount, tick } from "svelte";
  import { Avatar, AvatarImage, AvatarFallback } from "@arlen/ui-kit/components/ui/avatar";
  import type { AuthResult, Profile } from "$lib/greeter";
  import PasswordField from "./PasswordField.svelte";
  import FactorRow from "./FactorRow.svelte";

  let {
    profile,
    hasHardwareFactor = false,
    canSwitch = false,
    onsubmit,
    onfactor,
    onswitch,
  }: {
    profile: Profile;
    hasHardwareFactor?: boolean;
    canSwitch?: boolean;
    onsubmit: (secret: string) => Promise<AuthResult>;
    onfactor: () => Promise<AuthResult>;
    onswitch?: () => void;
  } = $props();

  let secret = $state("");
  let busy = $state(false);
  let waiting = $state(false);
  let error = $state<string | null>(null);
  let errored = $state(false);
  let fieldRef = $state<PasswordField | null>(null);

  onMount(() => fieldRef?.focus());

  function initials(name: string): string {
    return name.split(/\s+/).filter(Boolean).slice(0, 2).map((p) => p[0]?.toUpperCase() ?? "").join("");
  }

  async function fail(message: string | undefined) {
    error = message ?? "That did not work. Try again.";
    secret = "";
    errored = false;
    await tick();
    errored = true;
    fieldRef?.focus();
  }

  async function submit() {
    if (secret.length === 0 || busy) return;
    busy = true;
    error = null;
    errored = false;
    const r = await onsubmit(secret);
    busy = false;
    if (!r.ok) await fail(r.error);
  }

  async function factor() {
    waiting = true;
    error = null;
    const r = await onfactor();
    waiting = false;
    if (!r.ok) await fail(r.error);
  }
</script>

<div class="panel">
  <span class="avatar">
    <Avatar class="size-14">
      {#if profile.avatar_url}
        <AvatarImage src={profile.avatar_url} alt="" />
      {/if}
      <AvatarFallback class="bg-foreground/15 text-base font-semibold text-foreground">
        {initials(profile.name)}
      </AvatarFallback>
    </Avatar>
  </span>
  <span class="name">{profile.name}</span>

  {#if !waiting}
    <PasswordField bind:this={fieldRef} bind:value={secret} {busy} error={errored} onsubmit={submit} />
  {/if}

  <p class="error" role="alert" aria-live="assertive">{error ?? ""}</p>

  <FactorRow
    available={hasHardwareFactor}
    {waiting}
    onbegin={factor}
    oncancel={() => (waiting = false)}
  />

  {#if canSwitch && !waiting}
    <button type="button" class="switch" id="greeter-switch-user" onclick={() => onswitch?.()}>
      Switch user
    </button>
  {/if}
</div>

<style>
  /* The flat panel: the Waypointer / Quick Settings recipe. A subtle
     surface tint, a 1px hairline, the card radius, a quiet float shadow.
     No blur, no gradient. */
  .panel {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
    width: calc(20rem * var(--greeter-scale, 1));
    max-width: 90vw;
    padding: 1.5rem 1.25rem 1.25rem;
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    box-shadow: var(--shadow-md);
  }
  .avatar {
    display: inline-flex;
    color: var(--foreground);
  }
  .name {
    font-size: calc(1.0625rem * var(--greeter-scale, 1));
    font-weight: 500;
    color: var(--foreground);
  }
  /* The field spans the panel width. */
  .panel :global(.field) {
    width: 100%;
  }
  .error {
    margin: -0.25rem 0 0;
    min-height: 1rem;
    font-size: calc(0.75rem * var(--greeter-scale, 1));
    color: color-mix(in srgb, var(--color-error) 80%, white 20%);
    text-align: center;
  }
  .switch {
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: calc(0.75rem * var(--greeter-scale, 1));
  }
  .switch:hover {
    color: var(--foreground);
  }
</style>
