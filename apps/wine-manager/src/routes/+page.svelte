<script lang="ts">
  /// The bottle list: what exists, what each program can reach, and which bottle
  /// files would not parse. The last part is the reason this page is not a plain
  /// list: a bottle that is on disk and unreadable has to say so here, or the
  /// person goes looking for it in the filesystem.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n/messages";
  import { Wine } from "@lucide/svelte";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";

  type DriveView = { letter: string; host: string; writable: boolean };
  type HealthView = {
    agrees: boolean;
    missing: string[];
    unexpected: string[];
    escapes: string[];
    booted: boolean;
  };
  type BottleView = {
    id: string;
    prefix: string;
    drives: DriveView[];
    egress: string;
    health: HealthView | null;
  };
  type UnreadableBottle = { path: string; reason: string };
  type BottleList = { bottles: BottleView[]; unreadable: UnreadableBottle[] };
  type Runtime = { wine: boolean; bottles_dir: string | null };

  let list = $state<BottleList | null>(null);
  let repaired = $state<Record<string, string>>({});
  // Page-level, not per card. The forget notice used to live inside the card, and
  // the press removes that card - so the sentence saying where the files went was
  // rendered into an element that no longer existed. Nobody was told anything.
  let notice = $state<string | null>(null);
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
  <!-- The window is undecorated (`decorations: false`), so without this header
       there is no way to move, minimise or close it at all: the app shipped as a
       window a person could not put down. Every other app on this image carries
       the same bar. -->
  <header class="bar">
    <Wine size={16} strokeWidth={2} />
    <h1>{$t("wn.app.title")}</h1>
    <span class="spacer"></span>
    <WindowButtons />
  </header>

  {#if notice}
    <p class="notice">{notice}</p>
  {/if}
  {#if failure}
    <p class="failure">{$t("wn.failed", { reason: failure })}</p>
  {:else if list}
    {#if list.bottles.length === 0 && list.unreadable.length === 0}
      <p class="empty">{runtime && !runtime.wine ? $t("wn.noWine") : $t("wn.none")}</p>
      {#if runtime?.bottles_dir}
        <!-- Where, not just what. An empty window that only explains the idea
             leaves a person with nowhere to go; the calendar names its folder
             and this one now does too. -->
        <p class="empty">{$t("wn.whereBottles", { dir: runtime.bottles_dir })}</p>
      {/if}
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
                <button
                  class="revoke"
                  onclick={async () => {
                    try {
                      // The card is replaced from what came BACK, never from what
                      // the press asked for: a revoke that failed must not leave a
                      // window showing a folder as gone while the letter is there.
                      const after = await invoke<BottleView>("wine_revoke", {
                        id: bottle.id,
                        letter: drive.letter.replace(":", ""),
                      });
                      const at = list?.bottles.findIndex((b) => b.id === bottle.id) ?? -1;
                      if (list && at >= 0) list.bottles[at] = after;
                    } catch (e) {
                      repaired[bottle.id] = $t("wn.revokeFailed", {
                        letter: drive.letter,
                        reason: String(e),
                      });
                    }
                  }}>{$t("wn.revoke")}</button
                >
              </li>
            {/each}
          </ul>
        {/if}
        {#if bottle.health && !bottle.health.booted}
          <p class="warn">{$t("wn.notBooted")}</p>
        {:else if bottle.health && !bottle.health.agrees}
          {#if bottle.health.missing.length}
            <p class="warn">{$t("wn.driveMissing", { letters: bottle.health.missing.join(", ") })}</p>
          {/if}
          {#if bottle.health.unexpected.length}
            <p class="warn">
              {$t("wn.driveUnexpected", { letters: bottle.health.unexpected.join(", ") })}
            </p>
          {/if}
          {#if bottle.health.escapes.length}
            <p class="warn">{$t("wn.escaped", { paths: bottle.health.escapes.join(", ") })}</p>
          {/if}
          <p class="repair">
            <button
              onclick={async () => {
                try {
                  const after = await invoke<HealthView>("wine_repair", { id: bottle.id });
                  bottle.health = after;
                  repaired[bottle.id] = after.agrees ? $t("wn.repaired") : "";
                } catch (e) {
                  repaired[bottle.id] = $t("wn.repairFailed", { reason: String(e) });
                }
              }}>{$t("wn.repair")}</button
            >
            <span>{$t("wn.repairNote")}</span>
          </p>
        {/if}
        <p class="repair">
          <button
            onclick={async () => {
              try {
                const where = await invoke<string>("wine_forget", { id: bottle.id });
                // Drawn from what came back: the row goes only once the files
                // have actually moved.
                if (list) list.bottles = list.bottles.filter((b) => b.id !== bottle.id);
                notice = where
                  ? $t("wn.forgotten", { path: where })
                  : $t("wn.forgottenNoFiles");
              } catch (e) {
                // The bottle is still here, so this one belongs on its card.
                repaired[bottle.id] = $t("wn.forgetFailed", { reason: String(e) });
              }
            }}>{$t("wn.forget")}</button
          >
          <span>{$t("wn.forgetNote")}</span>
        </p>
        {#if repaired[bottle.id]}
          <p class="repaired">{repaired[bottle.id]}</p>
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
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--color-border-default, #2a2a2a);
  }
  .bar h1 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
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
  .repair {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
    flex-wrap: wrap;
    margin: 0.5rem 0 0;
  }
  .revoke {
    font: inherit;
    font-size: 0.8rem;
    padding: 0.1rem 0.5rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .repair button {
    font: inherit;
    padding: 0.25rem 0.7rem;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .notice {
    margin: 0;
    padding: 0.5rem 0.75rem;
    border-radius: 8px;
    border: 1px solid var(--border);
    opacity: 0.9;
  }
  .repaired {
    margin: 0.35rem 0 0;
    opacity: 0.8;
  }
  .warn {
    color: var(--destructive, #b3261e);
    font-size: 0.9rem;
    margin: 0.35rem 0 0;
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
