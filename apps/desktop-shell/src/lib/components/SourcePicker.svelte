<script lang="ts">
  /// The screencast source-picker (screenshot-capture-plan.md §3): a consent-framed
  /// chooser for "what do I share" when an app requests a screencast. Deny is
  /// first-class, only what you pick is sent, remembering is off by default. Mounted
  /// once in the shell layout beside the other request dialogs. Fixture-backed.
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Checkbox } from "@arlen/ui-kit/components/ui/checkbox";
  import { current, sources, share, cancel } from "$lib/stores/sourcePicker";

  let selected = $state<{ kind: "monitor" | "window" | "region"; id: string } | null>(null);
  let showCursor = $state(true);
  let remember = $state(false);

  function pick(kind: "monitor" | "window" | "region", id: string) {
    selected = { kind, id };
  }
  const isPicked = (kind: string, id: string) => selected?.kind === kind && selected.id === id;

  function doShare() {
    if (selected) void share({ ...selected, showCursor, remember });
  }
</script>

{#if $current}
  {@const req = $current}
  <Dialog.Root open={true} onOpenChange={(o) => { if (!o) cancel(); }}>
    <Dialog.Content>
      <div class="sp">
        <header class="sp-head">
          <h2 class="sp-title">{req.requesterLabel} wants to share your screen</h2>
          <p class="sp-sub">Choose what to share. Only what you pick is sent.</p>
        </header>

        <div class="sp-scroll">
          {#if $sources.monitors.length > 0}
            <div class="sp-group">Screens</div>
            <div class="sp-monitors">
              {#each $sources.monitors as m (m.id)}
                <button type="button" class="sp-mon" class:on={isPicked("monitor", m.id)} onclick={() => pick("monitor", m.id)}>
                  <span class="sp-mon-prev"></span>
                  <span class="sp-mon-name">{m.name}</span>
                  <span class="sp-mon-res">{m.resolution}</span>
                </button>
              {/each}
            </div>
          {/if}

          {#if $sources.windows.length > 0}
            <div class="sp-group">Windows</div>
            <div class="sp-wins">
              {#each $sources.windows as w (w.id)}
                <button type="button" class="sp-win" class:on={isPicked("window", w.id)} onclick={() => pick("window", w.id)}>
                  <span class="sp-avatar">{w.appLabel.charAt(0)}</span>
                  <span class="sp-win-text">
                    <span class="sp-win-app">{w.appLabel}</span>
                    <span class="sp-win-title">{w.title}</span>
                  </span>
                </button>
              {/each}
            </div>
          {/if}

          <div class="sp-group">Region</div>
          <button type="button" class="sp-region" class:on={isPicked("region", "region")} onclick={() => pick("region", "region")}>
            Choose a region to share
          </button>
        </div>

        <div class="sp-toggles">
          <div class="sp-toggle">
            <Checkbox id="sp-cursor" bind:checked={showCursor} ariaLabel="Show my cursor" />
            <label for="sp-cursor">Show my cursor</label>
          </div>
          <div class="sp-toggle">
            <Checkbox id="sp-remember" bind:checked={remember} ariaLabel={`Remember for ${req.requesterLabel}`} />
            <label for="sp-remember">Remember for {req.requesterLabel}</label>
          </div>
        </div>

        <footer class="sp-foot">
          <Button variant="outline" onclick={() => cancel()}>Don't share</Button>
          <span class="sp-spacer"></span>
          <Button onclick={doShare} disabled={!selected}>Share</Button>
        </footer>
      </div>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .sp {
    display: flex;
    flex-direction: column;
    max-height: min(80vh, 620px);
  }
  .sp-head {
    padding: 1.25rem 1.25rem 0.5rem;
  }
  .sp-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--foreground);
  }
  .sp-sub {
    margin: 0.2rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .sp-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.25rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  .sp-group {
    margin-top: 0.5rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
  }

  /* Screens: preview cards in a responsive row. The preview is a neutral block
     (a live thumbnail replaces it); the name labels the screen. */
  .sp-monitors {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(9rem, 1fr));
    gap: 0.5rem;
  }
  .sp-mon {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 0.5rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .sp-mon-prev {
    display: block;
    aspect-ratio: 16 / 10;
    border-radius: var(--radius-chip);
    background: linear-gradient(
      135deg,
      color-mix(in srgb, var(--foreground) 13%, transparent),
      color-mix(in srgb, var(--foreground) 6%, transparent)
    );
    margin-bottom: 0.3rem;
  }
  .sp-mon-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .sp-mon-res {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-variant-numeric: tabular-nums;
  }

  .sp-wins {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }
  .sp-win {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.5rem 0.625rem;
    border: 1px solid transparent;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .sp-avatar {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    flex-shrink: 0;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--foreground);
  }
  .sp-win-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .sp-win-app {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
  }
  .sp-win-title {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sp-region {
    padding: 0.6rem 0.625rem;
    border: 1px dashed var(--color-border-strong, var(--color-border));
    border-radius: var(--radius-input);
    background: transparent;
    text-align: left;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    cursor: pointer;
  }

  /* Selection: a foreground wash + a solid border, the ChoiceList language. */
  .sp-mon:hover,
  .sp-win:hover,
  .sp-region:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .sp-mon.on,
  .sp-win.on,
  .sp-region.on {
    border-color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }

  .sp-toggles {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 1.25rem 0;
  }
  .sp-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    align-self: flex-start;
    padding: 0.25rem 0;
  }
  .sp-toggle label {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    cursor: pointer;
  }

  .sp-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1.25rem 1.25rem;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    margin-top: 0.5rem;
  }
  .sp-spacer {
    flex: 1;
  }
</style>
