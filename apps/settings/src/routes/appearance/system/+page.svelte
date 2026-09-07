<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// System: cursor, icons, and the terminal palette. The terminal's 16-ANSI IS
  /// live-previewable (a mini terminal); cursor / icons are OS-level and can't
  /// be faked in a Settings webview, so they show the control + honest
  /// indicators, not a fake preview (same principle as GTK). Same split +
  /// override language. Rich by structure, not omission (appearance-surface.md).
  /// Sounds have their own page (/appearance/sound).
  ///
  /// Mock-vs-live: the biggest backend gap - cursor/icon theme listing + setting +
  /// generator, and terminal per-slot editing need coder backend. Fixture-backed
  /// until then.
  import { MousePointer2, Image } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import { onMount } from "svelte";
  import {
    overrides,
    effective,
    isOverridden,
    setSys,
    resetSys,
    resetTerminal,
    loadSys,
    sysWriteFailed,
    installedCursorThemes,
    installedIconThemes,
    sysOptions,
    ANSI_META,
    type SysOption,
  } from "$lib/stores/themeSystem";

  $effect(() => {
    // What `theme.toml` already holds, so a value set on an earlier launch is
    // shown as set instead of the page opening on the theme's own defaults.
    void loadSys();
  });

  // What this machine actually has. `undefined` is "not read yet" and `null` is
  // "could not be read"; neither is an empty list, which would say the machine
  // has no cursor themes at all.
  let cursors = $state<SysOption[] | null | undefined>(undefined);
  let icons = $state<SysOption[] | null | undefined>(undefined);
  onMount(() => {
    void installedCursorThemes().then((c) => (cursors = c));
    void installedIconThemes().then((i) => (icons = i));
  });

  const cursorSize = $derived(Number($effective.cursorSize));
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
    {#if $sysWriteFailed}
      <p class="note span-full" role="alert">{$t("s.sys.writeFailed")}</p>
    {/if}
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
            <!-- The list used to be four names written here, of which this
                 machine might have none. Since the theme's cursor name is
                 written into the GTK settings files, picking one that is not
                 installed sends every GTK app to a fallback while the row shows
                 a confident selection. Same fix the sound theme got. -->
            {#if cursors === undefined}
              <span class="sys-said">{$t("s.sys.listReading")}</span>
            {:else if cursors === null}
              <span class="sys-said">{$t("s.sys.listUnreadable")}</span>
            {:else}
              {#if !cursors.some((c) => c.value === String($effective.cursorTheme))}
                <span class="sys-said">{$t("s.sys.themeMissing")}</span>
              {/if}
              <PopoverSelect value={String($effective.cursorTheme)} options={sysOptions(cursors, $t)} ariaLabel={$t("s.sys.cursorTheme")} onchange={(v) => setSys("cursorTheme", v)} />
            {/if}
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
            <ValueSlider value={cursorSize} min={16} max={48} step={2} unit="px" ariaLabel={$t("s.sys.cursorSize")} onchange={(v) => setSys("cursorSize", v)} />
          {/snippet}
        </OverrideRow>
      </Section>

      <Section label={$t("s.sys.icons")}>
        <OverrideRow
          label={$t("s.sys.theme")}
          hint={$t("s.sys.iconHint")}
          overridden={isOverridden($overrides, "iconTheme")}
          onreset={() => resetSys("iconTheme")}
          id="sys-iconTheme"
        >
          {#snippet control()}
            {#if icons === undefined}
              <span class="sys-said">{$t("s.sys.listReading")}</span>
            {:else if icons === null}
              <span class="sys-said">{$t("s.sys.listUnreadable")}</span>
            {:else}
              {#if !icons.some((i) => i.value === iconTheme)}
                <span class="sys-said">{$t("s.sys.themeMissing")}</span>
              {/if}
              <PopoverSelect value={iconTheme} options={sysOptions(icons, $t)} ariaLabel={$t("s.sys.iconTheme")} onchange={(v) => setSys("iconTheme", v)} />
            {/if}
          {/snippet}
        </OverrideRow>
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
            <!-- "applied to the desktop" is a claim, and it was made whatever
                 the name was. The house theme's icon token defaults to
                 `default`, which is a cursor redirect rather than an icon
                 theme, so the settings writers correctly leave the key out and
                 nothing is applied - while this line said it was.
                 THREE states, because "Default" is a third thing and not a
                 missing theme: it is the choice to name none, and what then
                 happens is that apps keep whatever the system gives them. The
                 absent branch is for a name somebody set that this machine does
                 not have, which is the case that used to read as applied. -->
            <span class="ind-note">
              {#if iconTheme === "Default"}
                {$t("s.sys.indIconsDefault")}
              {:else if icons && !icons.some((i) => i.value === iconTheme)}
                {$t("s.sys.indIconsAbsent", { theme: iconTheme })}
              {:else}
                {$t("s.sys.indIcons", { theme: iconTheme })}
              {/if}
            </span>
          </div>
        </div>
      </div>
    </aside>
    </div>
  </SectionGrid>
</Page>

<style>
  /* The three answers a picker can give that are not a list: reading, could not
     read, and the configured one is not on this machine. */
  .sys-said {
    font-size: var(--font-size-sm);
    color: var(--color-fg-secondary);
  }

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
    color: var(--color-fg-secondary, #a1a1aa);
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
  }

  /* The swatch IS the control and the input inside it is invisible, so focus
     lands on something with nothing to show. A keyboard reaching this picker saw
     no sign of it at all; `:focus-within` puts the ring on the thing a person
     can see. Same treatment, same reason, wherever this pattern is written. */
  .ts-swatch:focus-within {
    outline: 2px solid var(--color-accent, var(--foreground));
    outline-offset: 2px;
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

</style>
