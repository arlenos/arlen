<script lang="ts">
  /// The profile chooser on a multi-user box: a centered row of avatars,
  /// one per profile, as a radio group on the dimmed wallpaper. Selecting
  /// one focuses it and the parent opens the password panel. Keyboard:
  /// arrows move, Enter or Space selects. Flat and monochrome, the house
  /// avatar treatment (squircle, foreground-tinted fill, no gradient).
  import { Avatar, AvatarImage, AvatarFallback } from "@arlen/ui-kit/components/ui/avatar";
  import type { Profile } from "$lib/greeter";

  let {
    profiles,
    selectedId = null,
    onselect,
  }: {
    profiles: Profile[];
    selectedId?: string | null;
    onselect: (id: string) => void;
  } = $props();

  function initials(name: string): string {
    return name.split(/\s+/).filter(Boolean).slice(0, 2).map((p) => p[0]?.toUpperCase() ?? "").join("");
  }

  function onkeydown(e: KeyboardEvent, i: number) {
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      onselect(profiles[(i + 1) % profiles.length].id);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      onselect(profiles[(i - 1 + profiles.length) % profiles.length].id);
    } else if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      onselect(profiles[i].id);
    }
  }
</script>

<div class="row" role="radiogroup" aria-label="Choose a profile">
  {#each profiles as p, i (p.id)}
    <button
      type="button"
      class="profile"
      class:selected={p.id === selectedId}
      role="radio"
      aria-checked={p.id === selectedId}
      id={`greeter-profile-${p.id}`}
      tabindex={p.id === selectedId || (selectedId === null && i === 0) ? 0 : -1}
      onclick={() => onselect(p.id)}
      onkeydown={(e) => onkeydown(e, i)}
    >
      <Avatar class="size-16">
        {#if p.avatar_url}
          <AvatarImage src={p.avatar_url} alt="" />
        {/if}
        <AvatarFallback class="bg-foreground/15 text-lg font-semibold text-foreground">
          {initials(p.name)}
        </AvatarFallback>
      </Avatar>
      <span class="name">{p.name}</span>
    </button>
  {/each}
</div>

<style>
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    justify-content: center;
    gap: 1rem;
  }
  /* Each profile is a flat selectable cell, the Waypointer-row treatment:
     transparent at rest, a subtle foreground tint on hover and selection. */
  .profile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    width: 7rem;
    padding: 0.875rem 0.5rem;
    border: 1px solid transparent;
    border-radius: var(--radius-card);
    background: transparent;
    color: var(--foreground);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }
  .profile:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .profile.selected {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 12%, transparent);
  }
  .profile:focus-visible {
    outline: none;
    border-color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .name {
    font-size: calc(0.875rem * var(--greeter-scale, 1));
    font-weight: 500;
  }
</style>
