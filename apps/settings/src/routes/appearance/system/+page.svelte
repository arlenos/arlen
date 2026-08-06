<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// System: cursor, icons, sounds, and the terminal palette. The terminal's
  /// 16-ANSI IS live-previewable (a mini terminal); cursor / icons / sounds are
  /// OS-level and can't be faked in a Settings webview, so they show the control +
  /// honest indicators, not a fake preview (same principle as GTK). Same split +
  /// override language. Rich by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: the biggest backend gap - cursor/icon theme listing + setting +
  /// generator, the sound map + playback, and terminal per-slot editing all need
  /// coder backend. Fixture-backed until then.
  import { ChevronRight, MousePointer2, Image } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
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
    sysOptions,
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
  title={$t("s.sys.title")}
  description={$t("s.sys.desc")}
>
  <SectionGrid>
    <div class="editor span-full">
    <div class="controls">
      <Section label={$t("s.sys.cursor")}>
        <OverrideRow
          label={$t("s.sys.theme")}
          hint={$t("s.sys.cursorThemeHint")}
          overridden={isOverridden($overrides, "cursorTheme")}
          onreset={() => resetSys("cursorTheme")}
          id="sys-cursorTheme"
        >
          {#snippet control()}
            <PopoverSelect value={String($effective.cursorTheme)} options={sysOptions(CURSOR_THEMES, $t)} ariaLabel={$t("s.sys.cursorTheme")} onchange={(v) => setSys("cursorTheme", v)} />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label={$t("s.sys.size")}
          hint={$t("s.sys.sizeHint")}
          overridden={isOverridden($overrides, "cursorSize")}
          onreset={() => resetSys("cursorSize")}
          id="sys-cursorSize"
        >
          {#snippet control()}
            <ValueSlider value={cursorSize} min={16} max={48} step={2} unit="px" ariaLabel="Cursor size" onchange={(v) => setSys("cursorSize", v)} />
          {/snippet}
        </OverrideRow>
      </Section>

      <Section label={$t("s.sys.icons")}>
        <OverrideRow
          label={$t("s.sys.theme")}
          hint="The icon set across your apps"
          overridden={isOverridden($overrides, "iconTheme")}
          onreset={() => resetSys("iconTheme")}
          id="sys-iconTheme"
        >
          {#snippet control()}
            <PopoverSelect value={iconTheme} options={sysOptions(ICON_THEMES, $t)} ariaLabel={$t("s.sys.iconTheme")} onchange={(v) => setSys("iconTheme", v)} />
          {/snippet}
        </OverrideRow>
      </Section>

      <Section label={$t("s.sys.sounds")}>
        <OverrideRow
          label={$t("s.sys.sysSounds")}
          hint={$t("s.sys.sysSoundsHint")}
          overridden={isOverridden($overrides, "soundsEnabled")}
          onreset={() => resetSys("soundsEnabled")}
          id="sys-soundsEnabled"
        >
          {#snippet control()}
            <Switch value={soundsOn} ariaLabel="System sounds" onchange={(v) => setSys("soundsEnabled", v)} />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label={$t("s.sys.soundTheme")}
          hint={$t("s.sys.soundThemeHint")}
          overridden={isOverridden($overrides, "soundTheme")}
          onreset={() => resetSys("soundTheme")}
          id="sys-soundTheme"
        >
          {#snippet control()}
            <PopoverSelect value={String($effective.soundTheme)} options={sysOptions(SOUND_THEMES, $t)} ariaLabel={$t("s.sys.soundTheme")} onchange={(v) => setSys("soundTheme", v)} />
          {/snippet}
        </OverrideRow>
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            {$t("s.sys.allSounds")}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <Section>
              {#each SOUND_EVENTS as ev (ev.key)}
                <OverrideRow
                  label={$t(ev.label)}
                  hint={$t(ev.hint)}
                  overridden={isOverridden($overrides, ev.key)}
                  onreset={() => resetSys(ev.key)}
                  id={`sys-${ev.key}`}
                >
                  {#snippet control()}
                    <!-- A play-preview belongs here; it returns when a Settings
                         command can ask the daemon to play a cue. Until then no
                         dead button. -->
                    <PopoverSelect value={String($effective[ev.key])} options={sysOptions(SOUND_NAMES, $t)} ariaLabel={$t("s.snd.pickAria", { event: $t(ev.label) })} onchange={(v) => setSys(ev.key, v)} />
                  {/snippet}
                </OverrideRow>
              {/each}
            </Section>
          </CollapsibleContent>
        </Collapsible>
      </Section>

      <Section label={$t("s.sys.terminal")}>
        <div class="term-editor">
          <div class="term-grid">
            {#each ANSI_META as a (a.key)}
              <label
                class="ts-swatch"
                class:overridden={isOverridden($overrides, a.key)}
                style={`background:${$effective[a.key]}`}
                title={$t(a.label)}
              >
                <input type="color" value={String($effective[a.key])} oninput={(e) => setSys(a.key, e.currentTarget.value)} aria-label={$t(a.label)} />
              </label>
            {/each}
          </div>
          <div class="term-fgbg">
            <label class="ts-swatch wide" class:overridden={isOverridden($overrides, "termFg")} style={`background:${termFg}`} title={$t("s.sys.foreground")}>
              <input type="color" value={termFg} oninput={(e) => setSys("termFg", e.currentTarget.value)} aria-label={$t("s.sys.termFg")} />
            </label>
            <span class="fgbg-label">{$t("s.sys.text")}</span>
            <label class="ts-swatch wide" class:overridden={isOverridden($overrides, "termBg")} style={`background:${termBg}`} title={$t("s.sys.background")}>
              <input type="color" value={termBg} oninput={(e) => setSys("termBg", e.currentTarget.value)} aria-label={$t("s.sys.termBg")} />
            </label>
            <span class="fgbg-label">{$t("s.sys.background")}</span>
            {#if termOverridden}
              <button class="term-reset" type="button" onclick={resetTerminal}>{$t("s.sys.resetColours")}</button>
            {/if}
          </div>
        </div>
      </Section>
    </div>

    <aside class="preview-col">
      <div class="preview-sticky">
        <span class="preview-label">{$t("s.sys.preview")}</span>

        <!-- The sample below is deliberately not translated: it demonstrates the
             colours, and the text it imitates - a prompt, `ls` output, a compiler
             error - is what a real terminal prints, in English, whatever the UI
             language. It is listed in `dev/i18n-baseline.tsv` so the gate agrees. -->
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
            <span class="ind-note">{$t("s.sys.indCursor", { size: cursorSize })}</span>
          </div>
          <div class="ind">
            <span class="icon-tiles">
              <span class="icon-tile"><Image size={16} strokeWidth={1.75} /></span>
              <span class="icon-tile"><Image size={16} strokeWidth={1.75} /></span>
              <span class="icon-tile"><Image size={16} strokeWidth={1.75} /></span>
            </span>
            <span class="ind-note">{$t("s.sys.indIcons", { theme: iconTheme })}</span>
          </div>
        </div>
      </div>
    </aside>
    </div>
  </SectionGrid>
</Page>

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-width: 0;
  }
  .preview-sticky {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .preview-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    padding-inline-start: 0.125rem;
  }
  .preview-col {
    order: -1;
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
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    margin-inline-end: 0.5rem;
  }
  .term-reset {
    margin-inline-start: auto;
    border: none;
    background: transparent;
    font-size: var(--text-xs);
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
    font-size: var(--text-xs);
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
    font-size: var(--text-2xs);
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
    font-size: var(--text-sm);
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
