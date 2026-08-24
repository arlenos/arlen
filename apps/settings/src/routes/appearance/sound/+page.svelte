<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Sound (sound-system-plan.md SO-R3): the theme, the master switch, the
  /// volume, and one row per event with audition, cue choice and its own off
  /// switch. Two owners meet here and the page keeps them straight: the
  /// enabled/theme/volume half is the notification daemon's config (the
  /// soundSettings store, its own honest states), the per-event cue names are
  /// theme values (the themeSystem override model, reset back to the theme).
  /// A preview goes through the daemon's own resolver, so what you hear is
  /// what the system would play - not this page's idea of it.
  import { Play } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import {
    overrides,
    effective,
    isOverridden,
    setSys,
    resetSys,
    loadSys,
    sysWriteFailed,
    SOUND_EVENTS,
    previewSound,
    type PreviewOutcome,
    type SoundThemeOption,
  } from "$lib/stores/themeSystem";
  import {
    sound,
    soundWriteFailed,
    loadSound,
    patchSound,
    setEventSilenced,
    eventSilenced,
    soundThemeOptions,
    soundCueNames,
  } from "$lib/stores/soundSettings";

  /// What the last preview of each event did, so a click that made no sound can
  /// say which kind of nothing it was. Cleared after a moment so the row does
  /// not keep a stale verdict.
  let previewed = $state<Record<string, PreviewOutcome>>({});

  /// `undefined` until the read answers; empty means the read happened and
  /// found none, which the picker says rather than inventing a list.
  let soundThemes = $state<SoundThemeOption[] | undefined>(undefined);
  let cues = $state<string[] | undefined>(undefined);

  $effect(() => {
    void loadSound();
    void soundThemeOptions().then((t) => (soundThemes = t));
    void soundCueNames().then((c) => (cues = c));
    // The cue names theme.toml already overrides, so an earlier launch's edit
    // shows as set instead of the page opening on the theme's own defaults.
    void loadSys();
  });

  async function play(eventKey: string, name: string) {
    const outcome = await previewSound(name);
    previewed = { ...previewed, [eventKey]: outcome };
    // Long enough to read, short enough not to look like row state.
    setTimeout(() => {
      const { [eventKey]: _drop, ...rest } = previewed;
      previewed = rest;
    }, 2500);
  }

  const cfg = $derived($sound.settings);
  const volumePercent = $derived(cfg ? Math.round(cfg.volume * 100) : 0);
</script>

<Page
  title={$t("s.snd.title")}
  description={$t("s.snd.desc")}
>
  <SectionGrid>
    <!-- Unavailability is said once, by the row standing where the controls
         would be - a banner repeating the same sentence said it twice. -->
    {#if $sound.mocked}
      <p class="note span-full">{$t("s.snd.mocked")}</p>
    {/if}
    {#if $soundWriteFailed}
      <p class="note span-full" role="alert">{$t("s.snd.writeFailed")}</p>
    {/if}
    {#if $sysWriteFailed}
      <p class="note span-full" role="alert">{$t("s.sys.writeFailed")}</p>
    {/if}

    <Section label={$t("s.sys.sounds")} class="span-full">
      {#if !cfg}
        <!-- No config, no controls: a switch over an unread value is a claim. -->
        <p class="empty">{$t("s.snd.unavailable")}</p>
      {:else}
        <Row id="sound-enabled" label={$t("s.sys.sysSounds")} description={$t("s.sys.sysSoundsHint")}>
          {#snippet control()}
            <Switch
              value={cfg.enabled}
              ariaLabel={$t("s.sys.sysSounds")}
              onchange={(v) => patchSound({ enabled: v })}
            />
          {/snippet}
        </Row>
        <Row id="sound-theme" label={$t("s.sys.soundTheme")} description={$t("s.sys.soundThemeHint")}>
          {#snippet control()}
            {#if soundThemes === undefined}
              <span class="snd-said">{$t("s.snd.themesReading")}</span>
            {:else if soundThemes.length === 0}
              <span class="snd-said">{$t("s.snd.themesNone")}</span>
            {:else}
              {#if !soundThemes.some((th) => th.id === cfg.theme)}
                <!-- The configured theme is not on this machine. Worth saying:
                     the resolver will find nothing and every cue falls through
                     to the synth, which otherwise just sounds like a different
                     theme. -->
                <span class="snd-said">{$t("s.snd.themeMissing")}</span>
              {/if}
              <PopoverSelect
                value={cfg.theme}
                options={soundThemes.map((th) => ({ value: th.id, label: th.name }))}
                ariaLabel={$t("s.sys.soundTheme")}
                onchange={(v) => patchSound({ theme: v })}
              />
            {/if}
          {/snippet}
        </Row>
        {#if cfg.theme === "arlen"}
          <!-- Stated as a fact about the set, not a disclaimer: it exists, it
               is level-matched, and nobody has listened through it yet. -->
          <p class="caveat">{$t("s.snd.caveat")}</p>
        {/if}
        <Row id="sound-volume" label={$t("s.snd.volume")} description={$t("s.snd.volumeHint")}>
          {#snippet control()}
            <ValueSlider
              value={volumePercent}
              min={0}
              max={100}
              step={5}
              unit="%"
              ariaLabel={$t("s.snd.volume")}
              onchange={(v) => patchSound({ volume: v / 100 })}
            />
          {/snippet}
        </Row>
      {/if}
    </Section>

    <Section label={$t("s.sys.allSounds")} class="span-full">
      {#each SOUND_EVENTS as ev (ev.key)}
        {@const silenced = eventSilenced(cfg, ev.key)}
        <OverrideRow
          label={$t(ev.label)}
          hint={$t(ev.hint)}
          overridden={isOverridden($overrides, ev.key)}
          onreset={() => resetSys(ev.key)}
          id={`sound-${ev.key}`}
        >
          {#snippet control()}
            <span class="snd-control" class:silenced>
              <button
                type="button"
                class="snd-play"
                aria-label={$t("s.snd.playAria", { event: $t(ev.label) })}
                disabled={silenced}
                onclick={() => play(ev.key, String($effective[ev.key]))}
              >
                <Play size={13} strokeWidth={2} />
              </button>
              {#if previewed[ev.key] && previewed[ev.key] !== "played"}
                <!-- Only ever shown when nothing was heard. "It played" is
                     reported by the speaker. -->
                <span class="snd-said">{$t(`s.snd.outcome.${previewed[ev.key]}`)}</span>
              {/if}
              {#if cues === undefined}
                <span class="snd-said">{$t("s.snd.themesReading")}</span>
              {:else if cues.length === 0}
                <span class="snd-said">{$t("s.snd.cuesNone")}</span>
              {:else}
                <PopoverSelect
                  value={String($effective[ev.key])}
                  options={cues.map((c) => ({ value: c, label: c }))}
                  ariaLabel={$t("s.snd.pickAria", { event: $t(ev.label) })}
                  disabled={silenced}
                  onchange={(v) => setSys(ev.key, v)}
                />
              {/if}
              {#if cfg}
                <Switch
                  value={!silenced}
                  ariaLabel={$t("s.snd.enableAria", { event: $t(ev.label) })}
                  onchange={(v) => setEventSilenced(ev.key, !v)}
                />
              {/if}
            </span>
          {/snippet}
        </OverrideRow>
      {/each}
    </Section>
  </SectionGrid>
</Page>

<style>
  .note {
    margin: 0;
    padding: 0 0.25rem 0.5rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  /* The unreviewed-set line rides inside the card, under the theme row. */
  .caveat {
    margin: 0;
    padding: 0.25rem 1rem 0.625rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  .snd-control {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  /* A silenced event keeps its row - the cue is still configured - but its
     audition and picker step back until the switch returns. */
  .snd-control.silenced .snd-play {
    opacity: 0.4;
    cursor: default;
  }
  .snd-play {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border: 1px solid var(--color-border-default, #2a2a2a);
    border-radius: 6px;
    background: transparent;
    color: var(--color-fg-secondary, #a3a3a3);
    cursor: pointer;
  }
  .snd-play:hover:not(:disabled) {
    color: var(--color-fg-primary, #fafafa);
  }
  .snd-said {
    font-size: 11px;
    color: var(--color-fg-disabled, #737373);
  }
</style>
