<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import {
    Keyboard as KeyboardIcon,
    Plus,
    Trash2,
    ArrowUp,
    ArrowDown,
  } from "lucide-svelte";

  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";

  /// Everything the UI needs from `[xkb_config]` except layouts + variants,
  /// which go through dedicated backend commands so the single- vs
  /// array-form migration stays in one place.
  interface XkbExtras {
    options: string;
    model: string;
    rules: string;
    repeat_rate: number;
    repeat_delay: number;
  }

  const EXTRAS_DEFAULT: XkbExtras = {
    options: "",
    model: "",
    rules: "",
    repeat_rate: 25,
    repeat_delay: 600,
  };

  let layouts = $state<string[]>([]);
  let variants = $state<string[]>([]);
  let extras = $state<XkbExtras>({ ...EXTRAS_DEFAULT });
  let loading = $state(true);
  let lastError = $state<string | null>(null);
  let showAddLayout = $state(false);
  let extrasSaveTimer: ReturnType<typeof setTimeout> | null = null;

  /// Friendly labels for common XKB layouts. More exhaustive picker lives
  /// behind `xkb_config.layouts` — users can hand-edit any identifier
  /// `/usr/share/X11/xkb/rules/evdev.lst` accepts. Kept short for the
  /// chooser to stay usable without a search box.
  // Deliberately NOT routed through the catalog. These are xkb layout
  // descriptions, and xkeyboard-config already ships translations of them; hand
  // writing a German name for each here would be inventing data that has a
  // canonical source. The right fix reads the localised description from
  // `evdev.lst` rather than translating this list. Noted in `coder-reports.md`.
  const LAYOUT_CHOICES: { value: string; label: string }[] = [
    { value: "us", label: "English (US)" },
    { value: "gb", label: "English (UK)" },
    { value: "de", label: "German" },
    { value: "de(nodeadkeys)", label: "German (no dead keys)" },
    { value: "fr", label: "French" },
    { value: "es", label: "Spanish" },
    { value: "it", label: "Italian" },
    { value: "ch", label: "Swiss" },
    { value: "pt", label: "Portuguese" },
    { value: "se", label: "Swedish" },
    { value: "dk", label: "Danish" },
    { value: "no", label: "Norwegian" },
    { value: "fi", label: "Finnish" },
    { value: "nl", label: "Dutch" },
    { value: "pl", label: "Polish" },
    { value: "cz", label: "Czech" },
    { value: "hu", label: "Hungarian" },
    { value: "ru", label: "Russian" },
    { value: "jp", label: "Japanese" },
  ];

  function layoutLabel(code: string): string {
    return LAYOUT_CHOICES.find((c) => c.value === code)?.label ?? code;
  }

  // `$derived`, not `const`: the translator is a store, so a constant would hold
  // English from import onward and a locale switch would never reach these rows.
  const COMMON_OPTIONS: { value: string; label: string; description: string }[] =
    $derived([
      {
        value: "caps:escape",
        label: $t("s.kbd.capsEscape"),
        description: $t("s.kbd.capsEscapeDesc"),
      },
      {
        value: "caps:swapescape",
        label: $t("s.kbd.capsSwap"),
        description: $t("s.kbd.capsSwapDesc"),
      },
      {
        value: "grp:alt_shift_toggle",
        label: $t("s.kbd.altShift"),
        description: $t("s.kbd.altShiftDesc"),
      },
      {
        value: "terminate:ctrl_alt_bksp",
        label: $t("s.kbd.terminateX"),
        description: $t("s.kbd.terminateXDesc"),
      },
      {
        value: "compose:ralt",
        label: $t("s.kbd.compose"),
        description: $t("s.kbd.composeDesc"),
      },
    ]);

  function parseOptions(s: string): Set<string> {
    return new Set(
      s
        .split(",")
        .map((p) => p.trim())
        .filter(Boolean)
    );
  }

  function formatOptions(set: Set<string>): string {
    return Array.from(set).filter(Boolean).join(",");
  }

  const activeOptions = $derived(parseOptions(extras.options));

  /// The "+ Add Layout" picker should not re-offer already-configured
  /// layouts — it's strictly an add action.
  const addableLayouts = $derived(
    LAYOUT_CHOICES.filter((c) => !layouts.includes(c.value))
  );

  function setOption(value: string, enabled: boolean): void {
    const next = new Set(activeOptions);
    if (enabled) {
      next.add(value);
    } else {
      next.delete(value);
    }
    extras = { ...extras, options: formatOptions(next) };
    scheduleExtrasSave();
  }

  async function reload(): Promise<void> {
    loading = true;
    try {
      const [l, v, raw] = await Promise.all([
        invoke<string[]>("keyboard_get_layouts"),
        invoke<string[]>("keyboard_get_variants"),
        invoke<Partial<XkbExtras> | null>("config_get", {
          file: "compositor",
          key: "xkb_config",
        }),
      ]);
      layouts = l.length > 0 ? l : ["us"];
      variants = v;
      extras = { ...EXTRAS_DEFAULT, ...(raw ?? {}) };
      lastError = null;
    } catch (e) {
      lastError = String(e);
    } finally {
      loading = false;
    }
  }

  async function saveLayouts(): Promise<void> {
    try {
      lastError = null;
      await invoke("keyboard_set_layouts", { layouts });
      // Keep variants aligned with the layouts list so XKB doesn't get
      // a mismatched pair on the next apply.
      await invoke("keyboard_set_variants", { variants });
    } catch (e) {
      lastError = String(e);
    }
  }

  function scheduleExtrasSave(): void {
    if (extrasSaveTimer) clearTimeout(extrasSaveTimer);
    extrasSaveTimer = setTimeout(saveExtrasNow, 300);
  }

  async function saveExtrasNow(): Promise<void> {
    if (extrasSaveTimer) {
      clearTimeout(extrasSaveTimer);
      extrasSaveTimer = null;
    }
    try {
      const payload: Partial<XkbExtras> = {
        repeat_rate: extras.repeat_rate,
        repeat_delay: extras.repeat_delay,
      };
      if (extras.options) payload.options = extras.options;
      if (extras.model) payload.model = extras.model;
      if (extras.rules) payload.rules = extras.rules;
      await invoke("config_set", {
        file: "compositor",
        key: "xkb_config",
        value: payload,
      });
      lastError = null;
    } catch (e) {
      lastError = String(e);
    }
  }

  function addLayout(value: string): void {
    if (!value || layouts.includes(value)) {
      showAddLayout = false;
      return;
    }
    layouts = [...layouts, value];
    // Keep variants array the same length as layouts.
    variants = [...variants, ""];
    showAddLayout = false;
    void saveLayouts();
  }

  function removeLayout(index: number): void {
    if (layouts.length <= 1) return;
    layouts = layouts.filter((_, i) => i !== index);
    variants = variants.filter((_, i) => i !== index);
    void saveLayouts();
  }

  function moveLayout(from: number, to: number): void {
    if (to < 0 || to >= layouts.length || to === from) return;
    const nextLayouts = [...layouts];
    const [pulled] = nextLayouts.splice(from, 1);
    nextLayouts.splice(to, 0, pulled);
    layouts = nextLayouts;
    if (variants.length > from) {
      const nextVariants = [...variants];
      const [pulledV] = nextVariants.splice(from, 1);
      nextVariants.splice(to, 0, pulledV);
      variants = nextVariants;
    }
    void saveLayouts();
  }

  function setRepeatRate(n: number): void {
    extras = { ...extras, repeat_rate: n };
    scheduleExtrasSave();
  }

  function setRepeatDelay(n: number): void {
    extras = { ...extras, repeat_delay: n };
    scheduleExtrasSave();
  }

  onMount(() => {
    void reload();
  });
</script>

<Page
  title={$t("s.kbd.title")}
  description={$t("s.kbd.desc")}
>
  <SectionGrid>
  <!-- Navigation card pointing at the shortcut editor. Hash anchors
       the Keyboard category so users landing here from a `Configure
       switch shortcut` link see the relevant bindings. -->
  <LinkCard
    href="/keyboard/shortcuts#cat-keyboard"
    title={$t("s.kbd.shortcuts")}
    description={$t("s.kbd.shortcutsDesc")}
  >
    {#snippet icon()}<KeyboardIcon size={20} strokeWidth={1.75} />{/snippet}
  </LinkCard>

  {#if lastError}
    <div
      class="span-full rounded-[var(--radius-chip)] border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive"
      title={lastError}
    >
      {$t("s.kbd.saveFailed")}
    </div>
  {/if}

  <Section label={$t("s.kbd.layouts")}>
    <div class="flex flex-col divide-y divide-border">
      {#each layouts as layout, i (layout + ":" + i)}
        <div class="flex items-center gap-2 px-4 py-2.5">
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <span class="text-sm font-medium">
                {layoutLabel(layout)}
              </span>
              {#if i === 0}
                <span
                  class="rounded-[var(--radius-chip)] bg-primary/15 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-primary"
                  title={$t("s.kbd.primaryHint")}
                >
                  {$t("s.kbd.primary")}
                </span>
              {/if}
            </div>
            <div class="text-xs text-muted-foreground font-mono">
              {layout}
            </div>
          </div>
          <Button
            variant="ghost"
            size="icon"
            onclick={() => moveLayout(i, i - 1)}
            disabled={i === 0 || loading}
            aria-label={$t("s.kbd.moveUp")}
          >
            <ArrowUp class="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onclick={() => moveLayout(i, i + 1)}
            disabled={i === layouts.length - 1 || loading}
            aria-label={$t("s.kbd.moveDown")}
          >
            <ArrowDown class="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onclick={() => removeLayout(i)}
            disabled={layouts.length <= 1 || loading}
            aria-label={$t("s.kbd.removeLayout")}
          >
            <Trash2 class="h-3.5 w-3.5" />
          </Button>
        </div>
      {/each}
      <div class="px-4 py-2.5">
        {#if showAddLayout}
          <PopoverSelect
            value=""
            options={addableLayouts}
            onchange={addLayout}
            placeholder={$t("s.kbd.selectLayout")}
            width="100%"
          />
        {:else}
          <Button
            variant="ghost"
            size="sm"
            onclick={() => (showAddLayout = true)}
            disabled={addableLayouts.length === 0 || loading}
          >
            <Plus class="me-1 h-3.5 w-3.5" />
            {$t("s.kbd.addLayout")}
          </Button>
        {/if}
      </div>
    </div>
  </Section>

  <Section label={$t("s.kbd.primaryVariant")}>
    <Row
      label={$t("s.kbd.variant")}
      description="Optional variant for the primary layout, e.g. 'dvorak' or 'colemak'."
    >
      {#snippet control()}
        <Input
          value={variants[0] ?? ""}
          placeholder={$t("s.kbd.noneVariant")}
          oninput={(e: Event) => {
            const v = (e.target as HTMLInputElement).value;
            const next = [...variants];
            while (next.length < layouts.length) next.push("");
            next[0] = v;
            variants = next;
            if (extrasSaveTimer) clearTimeout(extrasSaveTimer);
            extrasSaveTimer = setTimeout(() => {
              extrasSaveTimer = null;
              void saveLayouts();
            }, 300);
          }}
          class="w-40"
          disabled={loading}
        />
      {/snippet}
    </Row>
  </Section>

  <Section label={$t("s.kbd.options")}>
    {#each COMMON_OPTIONS as opt (opt.value)}
      <Row label={opt.label} description={opt.description}>
        {#snippet control()}
          <Switch
            value={activeOptions.has(opt.value)}
            onchange={(v) => setOption(opt.value, v)}
            disabled={loading}
          />
        {/snippet}
      </Row>
    {/each}
  </Section>

  <Section label={$t("s.kbd.keyRepeat")}>
    <!-- Both rows share the same NumberInput width so their triggers
         form a clean vertical grid. 180px fits the longer unit label
         ("chars/s") plus a four-digit value without cropping, and
         leaves the shorter "ms" row looking intentional rather than
         orphaned. -->
    <Row
      label={$t("s.kbd.repeatRate")}
      description={$t("s.kbd.repeatRateDesc")}
    >
      {#snippet control()}
        <NumberInput
          value={extras.repeat_rate}
          min={1}
          max={200}
          step={1}
          unit="chars/s"
          disabled={loading}
          ariaLabel={$t("s.kbd.repeatRate")}
          onchange={setRepeatRate}
        />
      {/snippet}
    </Row>
    <Row
      label={$t("s.kbd.repeatDelay")}
      description={$t("s.kbd.repeatDelayDesc")}
    >
      {#snippet control()}
        <NumberInput
          value={extras.repeat_delay}
          min={50}
          max={2000}
          step={10}
          unit="ms"
          disabled={loading}
          ariaLabel={$t("s.kbd.repeatDelay")}
          onchange={setRepeatDelay}
        />
      {/snippet}
    </Row>
  </Section>
  </SectionGrid>
</Page>
