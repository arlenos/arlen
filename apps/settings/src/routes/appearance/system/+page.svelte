<script lang="ts">
  /// System: cursor, icons, sounds, and the terminal palette. The terminal's
  /// 16-ANSI IS live-previewable (a mini terminal); cursor / icons / sounds are
  /// OS-level and can't be faked in a Settings webview, so they show the control +
  /// honest indicators, not a fake preview (same principle as GTK). Same split +
  /// override language. Rich by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: the biggest backend gap - cursor/icon theme listing + setting +
  /// generator, the sound map + playback, and terminal per-slot editing all need
  /// coder backend. Fixture-backed until then.
  import { ChevronRight, MousePointer2, Play, Image } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import {
    overrides,
    effective,
    isOverridden,
    setSys,
    resetSys,
    resetTerminal,
    CURSOR_THEMES,
    ICON_THEMES,
    SOUND_THEMES,
    SOUND_NAMES,
    SOUND_EVENTS,
    ANSI_META,
  } from "$lib/stores/themeSystem";

  const cursorSize = $derived(Number($effective.cursorSize));
  const soundsOn = $derived(Boolean($effective.soundsEnabled));
  const iconTheme = $derived(String($effective.iconTheme));
  const termFg = $derived(String($effective.termFg));
  const termBg = $derived(String($effective.termBg));

  const termOverridden = $derived(
    Object.keys($overrides).some((k) => k.startsWith("ansi") || k === "termFg" || k === "termBg"),
  );
</script>

<Page
  title="System"
  description="Cursor, icons, sounds, and the terminal colours. Change one and it overrides just that value, on top of the theme."
>
  <SectionGrid>
    <div class="editor span-full">
    <div class="controls">
      <Group label="Cursor">
        <OverrideRow
          label="Theme"
          hint="The pointer style across the desktop"
          overridden={isOverridden($overrides, "cursorTheme")}
          onreset={() => resetSys("cursorTheme")}
          id="sys-cursorTheme"
        >
          {#snippet control()}
            <PopoverSelect value={String($effective.cursorTheme)} options={CURSOR_THEMES} ariaLabel="Cursor theme" width="12rem" onchange={(v) => setSys("cursorTheme", v)} />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label="Size"
          hint="The pointer size"
          overridden={isOverridden($overrides, "cursorSize")}
          onreset={() => resetSys("cursorSize")}
          id="sys-cursorSize"
        >
          {#snippet control()}
            <ValueSlider value={cursorSize} min={16} max={48} step={2} unit="px" ariaLabel="Cursor size" onchange={(v) => setSys("cursorSize", v)} />
          {/snippet}
        </OverrideRow>
      </Group>

      <Group label="Icons">
        <OverrideRow
          label="Theme"
          hint="The icon set across your apps"
          overridden={isOverridden($overrides, "iconTheme")}
          onreset={() => resetSys("iconTheme")}
          id="sys-iconTheme"
        >
          {#snippet control()}
            <PopoverSelect value={iconTheme} options={ICON_THEMES} ariaLabel="Icon theme" width="12rem" onchange={(v) => setSys("iconTheme", v)} />
          {/snippet}
        </OverrideRow>
      </Group>

      <Group label="Sounds">
        <OverrideRow
          label="System sounds"
          hint="Play a sound on system events"
          overridden={isOverridden($overrides, "soundsEnabled")}
          onreset={() => resetSys("soundsEnabled")}
          id="sys-soundsEnabled"
        >
          {#snippet control()}
            <Switch value={soundsOn} ariaLabel="System sounds" onchange={(v) => setSys("soundsEnabled", v)} />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label="Sound theme"
          hint="The set of system sounds"
          overridden={isOverridden($overrides, "soundTheme")}
          onreset={() => resetSys("soundTheme")}
          id="sys-soundTheme"
        >
          {#snippet control()}
            <PopoverSelect value={String($effective.soundTheme)} options={SOUND_THEMES} ariaLabel="Sound theme" width="12rem" onchange={(v) => setSys("soundTheme", v)} />
          {/snippet}
        </OverrideRow>
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            All sounds
          </CollapsibleTrigger>
          <CollapsibleContent>
            <Group>
              {#each SOUND_EVENTS as ev (ev.key)}
                <OverrideRow
                  label={ev.label}
                  hint={ev.hint}
                  overridden={isOverridden($overrides, ev.key)}
                  onreset={() => resetSys(ev.key)}
                  id={`sys-${ev.key}`}
                >
                  {#snippet control()}
                    <span class="snd-control">
                      <button class="snd-play" type="button" title="Preview (coming with audio)" aria-label={`Play ${ev.label}`}>
                        <Play size={13} strokeWidth={2} />
                      </button>
                      <PopoverSelect value={String($effective[ev.key])} options={SOUND_NAMES} ariaLabel={`${ev.label} sound`} width="10rem" onchange={(v) => setSys(ev.key, v)} />
                    </span>
                  {/snippet}
                </OverrideRow>
              {/each}
            </Group>
          </CollapsibleContent>
        </Collapsible>
      </Group>

      <Group label="Terminal">
        <div class="term-editor">
          <div class="term-grid">
            {#each ANSI_META as a (a.key)}
              <label
                class="ts-swatch"
                class:overridden={isOverridden($overrides, a.key)}
                style={`background:${$effective[a.key]}`}
                title={a.label}
              >
                <input type="color" value={String($effective[a.key])} oninput={(e) => setSys(a.key, e.currentTarget.value)} aria-label={a.label} />
              </label>
            {/each}
          </div>
          <div class="term-fgbg">
            <label class="ts-swatch wide" class:overridden={isOverridden($overrides, "termFg")} style={`background:${termFg}`} title="Foreground">
              <input type="color" value={termFg} oninput={(e) => setSys("termFg", e.currentTarget.value)} aria-label="Terminal foreground" />
            </label>
            <span class="fgbg-label">Text</span>
            <label class="ts-swatch wide" class:overridden={isOverridden($overrides, "termBg")} style={`background:${termBg}`} title="Background">
              <input type="color" value={termBg} oninput={(e) => setSys("termBg", e.currentTarget.value)} aria-label="Terminal background" />
            </label>
            <span class="fgbg-label">Background</span>
            {#if termOverridden}
              <button class="term-reset" type="button" onclick={resetTerminal}>Reset colours</button>
            {/if}
          </div>
        </div>
      </Group>
    </div>

    <aside class="preview-col">
      <div class="preview-sticky">
        <span class="preview-label">Live preview</span>

        <div class="term-preview" style={`background:${termBg}; color:${termFg}`}>
          <div class="tp-line">
            <span style={`color:${$effective.ansi2}`}>arlen@desktop</span><span>:</span><span style={`color:${$effective.ansi4}`}>~/src</span><span>$ ls --color</span>
          </div>
          <div class="tp-line">
            <span style={`color:${$effective.ansi4}`}>docs</span>
            <span style={`color:${$effective.ansi2}`}>src</span>
            <span style={`color:${$effective.ansi6}`}>build</span>
            <span>README.md</span>
          </div>
          <div class="tp-line"><span style={`color:${$effective.ansi1}`}>error:</span> <span>build failed</span></div>
          <div class="tp-line"><span style={`color:${$effective.ansi3}`}>warning:</span> <span style={`color:${$effective.ansi5}`}>deprecated</span> call</div>
          <div class="tp-swatchrow">
            {#each ANSI_META as a (a.key)}<span style={`background:${$effective[a.key]}`}></span>{/each}
          </div>
        </div>

        <div class="sys-indicators">
          <div class="ind">
            <MousePointer2 size={cursorSize} strokeWidth={1.75} />
            <span class="ind-note">Cursor, {cursorSize}px</span>
          </div>
          <div class="ind">
            <span class="icon-tiles">
              <span class="icon-tile"><Image size={16} strokeWidth={1.75} /></span>
              <span class="icon-tile"><Image size={16} strokeWidth={1.75} /></span>
              <span class="icon-tile"><Image size={16} strokeWidth={1.75} /></span>
            </span>
            <span class="ind-note">Icons: {iconTheme}, applied to the desktop</span>
          </div>
        </div>
      </div>
    </aside>
    </div>
  </SectionGrid>
</Page>

<style>
  .editor {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 20rem;
    gap: 1.5rem;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-width: 0;
  }
  .preview-sticky {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .preview-label {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    padding-left: 0.125rem;
  }
  @media (max-width: 60rem) {
    .editor {
      grid-template-columns: 1fr;
    }
    .preview-col {
      order: -1;
    }
    .preview-sticky {
      position: static;
    }
  }

  .snd-control {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .snd-play {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  .snd-play:hover {
    color: var(--foreground);
  }

  /* Terminal palette editor: a grid of 16 swatches + fg/bg. */
  .term-editor {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    padding: 0.75rem 1rem;
  }
  .term-grid {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 0.375rem;
    max-width: 22rem;
  }
  .ts-swatch {
    position: relative;
    height: 1.75rem;
    border-radius: var(--radius-button, 6px);
    border: 1px solid color-mix(in srgb, var(--foreground) 18%, transparent);
    cursor: pointer;
    overflow: hidden;
  }
  .ts-swatch.overridden {
    outline: 2px solid var(--color-accent, var(--foreground));
    outline-offset: 1px;
  }
  .ts-swatch input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .term-fgbg {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .ts-swatch.wide {
    width: 2.5rem;
    height: 1.5rem;
  }
  .fgbg-label {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    margin-right: 0.5rem;
  }
  .term-reset {
    margin-left: auto;
    border: none;
    background: transparent;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
  }
  .term-reset:hover {
    color: var(--foreground);
  }

  /* Terminal preview. */
  .term-preview {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.75rem;
    border-radius: var(--radius-card, 12px);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
    line-height: 1.5;
  }
  .tp-swatchrow {
    display: flex;
    gap: 2px;
    margin-top: 0.375rem;
  }
  .tp-swatchrow span {
    flex: 1;
    height: 0.5rem;
    border-radius: 2px;
  }

  .sys-indicators {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }
  .ind {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .ind-note {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .icon-tiles {
    display: inline-flex;
    gap: 0.25rem;
  }
  .icon-tile {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-button, 6px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* The expander trigger (class rides the Collapsible root, so global). */
  :global(.exp-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: 0.8125rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  :global(.exp-trigger:hover) {
    color: var(--foreground);
  }
  :global(.exp-trigger svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.exp-trigger[data-state="open"] svg) {
    transform: rotate(90deg);
  }
</style>
