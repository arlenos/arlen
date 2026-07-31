<script lang="ts">
  /// The store's places rail: Browse, Installed, Updates - the three things the
  /// store is for. The update count is a quiet number, never a red dot
  /// (update-flow-plan.md U-5: no nagging; routine updates wait to be looked at).
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/stores";
  import { Compass, Package, ArrowDownToLine } from "lucide-svelte";
  import { t } from "$lib/i18n/messages";
  import { updateCount, loadUpdates } from "$lib/stores/updates";

  onMount(loadUpdates);

  const PLACES = [
    { href: "/", labelKey: "st.rail.browse", icon: Compass },
    { href: "/installed", labelKey: "st.rail.installed", icon: Package },
    { href: "/updates", labelKey: "st.rail.updates", icon: ArrowDownToLine },
  ];
  const current = $derived($page.url.pathname);
</script>

<nav class="rail" aria-label={$t("st.title")}>
  {#each PLACES as p (p.href)}
    {@const Icon = p.icon}
    <button
      type="button"
      class="place"
      class:active={current === p.href}
      aria-current={current === p.href ? "page" : undefined}
      id={`rail-${p.href === "/" ? "browse" : p.href.slice(1)}`}
      onclick={() => goto(p.href)}
    >
      <Icon size={16} strokeWidth={1.75} />
      <span class="place-label">{$t(p.labelKey)}</span>
      {#if p.href === "/updates" && $updateCount > 0}
        <span class="count">{$updateCount}</span>
      {/if}
    </button>
  {/each}
</nav>

<style>
  .rail {
    flex: 0 0 11rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.75rem 0.6rem;
    border-right: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .place {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    padding: 0.4rem 0.5rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 82%, transparent);
    cursor: pointer;
  }
  .place:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .place.active {
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    color: var(--color-fg-primary);
    font-weight: 500;
  }
  .place-label {
    flex: 1;
  }
  .count {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
    font-variant-numeric: tabular-nums;
  }
</style>
