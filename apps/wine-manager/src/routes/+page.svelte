<script lang="ts">
  /// The bottle list: what exists, what each program can reach, and which bottle
  /// files would not parse. The last part is the reason this page is not a plain
  /// list: a bottle that is on disk and unreadable has to say so here, or the
  /// person goes looking for it in the filesystem.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n/messages";

  type DriveView = { letter: string; host: string; writable: boolean };
  type BottleView = { id: string; prefix: string; drives: DriveView[]; egress: string };
  type UnreadableBottle = { path: string; reason: string };
  type BottleList = { bottles: BottleView[]; unreadable: UnreadableBottle[] };
  type Runtime = { wine: boolean };

  let list = $state<BottleList | null>(null);
  let runtime = $state<Runtime | null>(null);
  let failure = $state<string | null>(null);

  onMount(async () => {
    try {
      list = await invoke<BottleList>("wine_bottles");
      runtime = await invoke<Runtime>("wine_runtime");
    } catch (e) {
      failure = String(e);
    }
  });
</script>

<main>
  {#if failure}
    <p class="failure">{$t("wn.failed", { reason: failure })}</p>
  {:else if list}
    {#if list.bottles.length === 0 && list.unreadable.length === 0}
      <p class="empty">{runtime && !runtime.wine ? $t("wn.noWine") : $t("wn.none")}</p>
    {/if}
    {#each list.bottles as bottle (bottle.id)}
      <section>
        <h2>{bottle.id}</h2>
        <p class="path"><span>{$t("wn.prefix")}</span> <code>{bottle.prefix}</code></p>
        <h3>{$t("wn.drives")}</h3>
        {#if bottle.drives.length === 0}
          <p class="none">{$t("wn.noDrives")}</p>
        {:else}
          <ul>
            {#each bottle.drives as drive (drive.letter)}
              <li>
                <code>{drive.letter}</code>
                <code>{drive.host}</code>
                <span>{drive.writable ? $t("wn.writable") : $t("wn.readOnly")}</span>
              </li>
            {/each}
          </ul>
        {/if}
        <p class="egress">
          <span>{$t("wn.egress")}</span>
          {bottle.egress === "none" ? $t("wn.egressNone") : bottle.egress}
        </p>
      </section>
    {/each}
    {#each list.unreadable as broken (broken.path)}
      <p class="broken">{$t("wn.unreadable", { path: broken.path, reason: broken.reason })}</p>
    {/each}
  {/if}
</main>

<style>
  main {
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  section {
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 0.75rem 1rem;
  }
  h2 {
    margin: 0 0 0.5rem;
    font-size: 1.05rem;
  }
  h3 {
    margin: 0.75rem 0 0.35rem;
    font-size: 0.85rem;
    font-weight: 500;
    opacity: 0.75;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  li {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
    min-width: 0;
  }
  li code:nth-of-type(2) {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  li span,
  .egress span,
  .path span {
    opacity: 0.7;
    font-size: 0.85rem;
  }
  .broken,
  .failure {
    color: var(--destructive, #b3261e);
  }
  .empty,
  .none {
    opacity: 0.75;
  }
</style>
