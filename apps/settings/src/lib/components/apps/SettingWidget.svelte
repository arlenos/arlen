<script lang="ts">
  /// One declared setting rendered as the right control for its type. The
  /// broker owns validation (schema, scope, bounds): every change goes through
  /// `writeKey`, and a refusal shows the broker's message at the row while the
  /// value snaps back to what the broker last served. The row never asserts.
  ///
  /// Type map: bool -> Switch, int/float/duration -> NumberInput, string ->
  /// Input (commit on blur/Enter), enum -> PopoverSelect (dynamic sources
  /// resolve through the broker; an unresolved source states why), string_list
  /// -> ChipList, path -> Input + Browse (picker pending), color -> swatch,
  /// keybind -> capture field, secret_ref -> set/replace (never the value),
  /// handoff -> a launch row into the app's own window, raw -> the validated
  /// TOML editor.
  import { invoke } from "@tauri-apps/api/core";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { PopoverSelect, type PopoverSelectOption } from "@arlen/ui-kit/components/ui/popover-select";
  import { ChipList } from "@arlen/ui-kit/components/ui/chip-list";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Textarea } from "@arlen/ui-kit/components/ui/textarea";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { ExternalLink, Folder } from "lucide-svelte";
  import type { SettingsItem, SettingOption } from "$lib/appSettings";
  import { writeKey, resetKey, writeRaw, resolveOptions } from "$lib/stores/appSettings";
  import { t } from "$lib/i18n/messages";

  let {
    appId,
    item,
    value,
    userSet = false,
    unavailableReason,
    error,
  }: {
    appId: string;
    item: SettingsItem;
    value: unknown;
    /// The user chose this value (vs the shipped default); enables Reset.
    userSet?: boolean;
    /// The broker's reason a dynamic source did not resolve.
    unavailableReason?: string;
    /// The broker's refusal message for the last write to this key.
    error?: string;
  } = $props();

  const rowId = $derived(`${appId}.${item.key}`);
  const showReset = $derived(userSet && item.default !== undefined && item.type !== "secret_ref");

  // bool: the kit Switch self-toggles its bindable, so a local mirror synced
  // from the store keeps a broker snap-back authoritative.
  let boolVal = $state(false);
  $effect(() => {
    boolVal = value === true || value === "true";
  });

  // string / path: draft locally, commit on blur or Enter so half-typed text
  // never hits the broker.
  let textDraft = $state("");
  $effect(() => {
    textDraft = typeof value === "string" ? value : value == null ? "" : String(value);
  });
  function commitText() {
    if (textDraft !== value) writeKey(item.key, textDraft);
  }

  let listItems = $state<string[]>([]);
  $effect(() => {
    listItems = Array.isArray(value) ? value.map(String) : [];
  });

  const numVal = $derived(typeof value === "number" ? value : Number(value ?? item.default ?? 0));

  // enum: declared options, or the broker-resolved list for a dynamic source.
  // Resolution state clears when the rendered item changes, so a re-used
  // component instance never shows the previous key's choices.
  let resolved = $state<SettingOption[] | null>(null);
  let resolveFailed = $state(false);
  $effect(() => {
    void item;
    resolved = null;
    resolveFailed = false;
  });
  $effect(() => {
    if (item.type === "enum" && item.options_from && !unavailableReason && (item.options ?? []).length === 0) {
      resolveOptions(item.options_from)
        .then((o) => (resolved = o))
        .catch(() => (resolveFailed = true));
    }
  });
  const enumOptions = $derived<SettingOption[]>(
    (item.options ?? []).length > 0 ? (item.options as SettingOption[]) : (resolved ?? [])
  );
  // The per-option description is mandatory by contract and the list renders
  // it; the collapsed trigger stays one line (the kit handles both).
  const selectOptions = $derived<PopoverSelectOption[]>(
    enumOptions.map((o) => ({ value: o.value, label: o.label, description: o.description }))
  );
  const enumBlocked = $derived(Boolean(unavailableReason) || resolveFailed);

  // keybind: focus the field, press the combo; Esc cancels the capture.
  let capturing = $state(false);
  function onCaptureKey(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      capturing = false;
      return;
    }
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
    const parts: string[] = [];
    if (e.ctrlKey) parts.push("Ctrl");
    if (e.altKey) parts.push("Alt");
    if (e.shiftKey) parts.push("Shift");
    if (e.metaKey) parts.push("Super");
    parts.push(e.key.length === 1 ? e.key.toUpperCase() : e.key);
    capturing = false;
    writeKey(item.key, parts.join("+"));
  }
  const keybindParts = $derived(typeof value === "string" && value.length > 0 ? value.split("+") : []);

  // secret_ref: the stored value is a vault reference; the row only ever says
  // whether one is set, and Replace sends new text for the broker to vault.
  const secretSet = $derived(typeof value === "string" && value.length > 0);
  let secretEditing = $state(false);
  let secretDraft = $state("");
  function saveSecret() {
    if (secretDraft.length > 0) writeKey(item.key, secretDraft);
    secretDraft = "";
    secretEditing = false;
  }

  // raw: apply-gated, the broker parses; its message shows inline.
  let rawDraft = $state("");
  let rawError = $state<string | null>(null);
  $effect(() => {
    rawDraft = typeof value === "string" ? value : "";
  });
  const rawDirty = $derived(rawDraft !== (typeof value === "string" ? value : ""));
  async function applyRaw() {
    rawError = await writeRaw(item.key, rawDraft);
  }

  let handoffFailed = $state(false);
  async function openHandoff() {
    try {
      await invoke("app_settings_handoff", { appId, window: item.handoff?.window ?? "" });
      handoffFailed = false;
    } catch {
      handoffFailed = true;
    }
  }

  let pickFailed = $state(false);
  async function browse() {
    try {
      const picked = await invoke<string | null>("settings_pick_path", {});
      if (picked) writeKey(item.key, picked);
      pickFailed = false;
    } catch {
      pickFailed = true;
    }
  }

  const wide = $derived(item.type === "string_list" || item.type === "raw");
  const hasBelow = $derived(
    wide || Boolean(error) || Boolean(item.deprecated_message) || enumBlocked || handoffFailed || pickFailed
  );
</script>

<Row id={rowId} label={item.label} description={item.description ?? undefined}>
  {#snippet control()}
    <div class="ctl">
      {#if showReset}
        <button type="button" class="reset" onclick={() => resetKey(item.key, item.default)}>
          {$t("s.apps.reset")}
        </button>
      {/if}

      {#if item.type === "bool"}
        <Switch bind:value={boolVal} ariaLabel={item.label} onchange={(v) => writeKey(item.key, v)} />
      {:else if item.type === "int" || item.type === "duration"}
        <NumberInput
          value={Math.round(numVal)}
          min={item.min ?? undefined}
          max={item.max ?? undefined}
          unit={item.unit ?? undefined}
          ariaLabel={item.label}
          onchange={(v) => writeKey(item.key, Math.round(v))}
        />
      {:else if item.type === "float"}
        <NumberInput
          value={numVal}
          min={item.min ?? undefined}
          max={item.max ?? undefined}
          step={0.5}
          unit={item.unit ?? undefined}
          ariaLabel={item.label}
          onchange={(v) => writeKey(item.key, v)}
        />
      {:else if item.type === "string"}
        <Input
          class="field"
          value={textDraft}
          oninput={(e: Event) => (textDraft = (e.currentTarget as HTMLInputElement).value)}
          onblur={commitText}
          onkeydown={(e: KeyboardEvent) => e.key === "Enter" && commitText()}
          aria-label={item.label}
        />
      {:else if item.type === "enum"}
        <PopoverSelect
          value={typeof value === "string" ? value : String(item.default ?? "")}
          options={selectOptions}
          disabled={enumBlocked}
          placeholder={enumBlocked ? $t("s.apps.noChoices") : undefined}
          ariaLabel={item.label}
          onchange={(v) => writeKey(item.key, v)}
        />
      {:else if item.type === "path"}
        <div class="path-ctl">
          <Input
            class="field"
            value={textDraft}
            oninput={(e: Event) => (textDraft = (e.currentTarget as HTMLInputElement).value)}
            onblur={commitText}
            onkeydown={(e: KeyboardEvent) => e.key === "Enter" && commitText()}
            aria-label={item.label}
          />
          <Button variant="outline" size="icon" aria-label={$t("s.apps.browse")} onclick={browse}>
            <Folder size={15} strokeWidth={1.75} />
          </Button>
        </div>
      {:else if item.type === "color"}
        <label class="swatch" style={`background:${typeof value === "string" ? value : "#000000"}`}>
          <input
            type="color"
            value={typeof value === "string" ? value : "#000000"}
            oninput={(e) => writeKey(item.key, e.currentTarget.value)}
            aria-label={item.label}
          />
        </label>
      {:else if item.type === "keybind"}
        {#if capturing}
          <button type="button" class="keybind capturing" onkeydown={onCaptureKey} onblur={() => (capturing = false)}>
            {$t("s.apps.keybindPrompt")}
          </button>
        {:else}
          <button type="button" class="keybind" onclick={() => (capturing = true)} aria-label={item.label}>
            {#each keybindParts as part, i (i)}
              <kbd>{part}</kbd>
            {:else}
              <span class="keybind-unset">{$t("s.apps.keybindUnset")}</span>
            {/each}
          </button>
        {/if}
      {:else if item.type === "secret_ref"}
        {#if secretEditing}
          <div class="secret-edit">
            <Input
              class="field"
              type="password"
              value={secretDraft}
              oninput={(e: Event) => (secretDraft = (e.currentTarget as HTMLInputElement).value)}
              onkeydown={(e: KeyboardEvent) => e.key === "Enter" && saveSecret()}
              aria-label={item.label}
            />
            <Button variant="outline" size="sm" onclick={saveSecret}>{$t("s.apps.secretSave")}</Button>
            <Button
              variant="ghost"
              size="sm"
              onclick={() => {
                secretDraft = "";
                secretEditing = false;
              }}
            >
              {$t("s.apps.cancel")}
            </Button>
          </div>
        {:else}
          <span class="secret-state" class:unset={!secretSet}>
            {secretSet ? $t("s.apps.secretSet") : $t("s.apps.secretNotSet")}
          </span>
          <Button variant="outline" size="sm" onclick={() => (secretEditing = true)}>
            {secretSet ? $t("s.apps.secretReplace") : $t("s.apps.secretAdd")}
          </Button>
        {/if}
      {:else if item.type === "handoff"}
        <Button variant="outline" size="sm" onclick={openHandoff}>
          <ExternalLink size={14} strokeWidth={1.75} />
          {$t("s.apps.handoffOpen")}
        </Button>
      {:else if item.type !== "string_list" && item.type !== "raw"}
        <!-- A type this Settings build does not know renders as an honest
             statement, never a silently empty cell. -->
        <span class="unknown-type">{$t("s.apps.unknownType")}</span>
      {/if}
    </div>
  {/snippet}

  {#snippet below()}
    {#if hasBelow}
      <div class="below">
        {#if item.type === "string_list"}
          <ChipList bind:items={listItems} onchange={(v) => writeKey(item.key, v)} id={rowId + ".list"} />
        {:else if item.type === "raw"}
          <div class="raw">
            <Textarea
              class="raw-text"
              value={rawDraft}
              oninput={(e: Event) => (rawDraft = (e.currentTarget as HTMLTextAreaElement).value)}
              rows={4}
              spellcheck={false}
              aria-label={item.label}
            />
            <div class="raw-foot">
              {#if rawError}
                <span class="err">{rawError}</span>
              {/if}
              <Button variant="outline" size="sm" disabled={!rawDirty} onclick={applyRaw}>
                {$t("s.apps.rawApply")}
              </Button>
            </div>
          </div>
        {/if}
        {#if error}
          <p class="err">{error}</p>
        {/if}
        {#if enumBlocked}
          <p class="note">{unavailableReason ?? $t("s.apps.optionsFail")}</p>
        {/if}
        {#if handoffFailed}
          <p class="note">{$t("s.apps.handoffFail")}</p>
        {/if}
        {#if pickFailed}
          <p class="note">{$t("s.apps.pickFail")}</p>
        {/if}
        {#if item.deprecated_message}
          <p class="note">{item.deprecated_message}</p>
        {/if}
      </div>
    {/if}
  {/snippet}
</Row>

<style>
  .ctl {
    display: flex;
    align-items: center;
    gap: 0.625rem;
  }
  .below {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  /* Reset is the quiet tidy action next to the control, present only where a
     user-chosen value can honestly return to a shipped default. */
  .reset {
    border: none;
    background: transparent;
    padding: 0.125rem 0.25rem;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
    transition: color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .reset:hover {
    color: var(--foreground);
  }

  .path-ctl {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    width: var(--control-width, 14rem);
  }
  .path-ctl :global(.field) {
    flex: 1;
    min-width: 0;
  }

  /* The colour swatch hides the native picker input under itself (the
     appearance page's settled pattern). */
  .swatch {
    position: relative;
    width: 1.75rem;
    height: 1.75rem;
    border-radius: var(--radius-button, 6px);
    border: 1px solid color-mix(in srgb, var(--foreground) 18%, transparent);
    cursor: pointer;
    overflow: hidden;
  }
  .swatch input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }

  .keybind {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    min-height: var(--height-control, 28px);
    padding: 0.125rem 0.5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-input, 6px);
    background: var(--input, transparent);
    font-size: var(--text-xs);
    color: var(--foreground);
    cursor: pointer;
  }
  .keybind.capturing {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    border-color: var(--ring, var(--border));
  }
  .keybind kbd {
    padding: 0.0625rem 0.3125rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-family: var(--font-mono, monospace);
    font-size: var(--text-2xs);
  }
  .keybind-unset {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }

  .secret-edit {
    display: flex;
    align-items: center;
    gap: 0.375rem;
  }
  .secret-state {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .unknown-type {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .secret-state.unset {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }

  .raw {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }
  .raw :global(.raw-text) {
    font-family: var(--font-mono, monospace);
    font-size: var(--text-xs);
  }
  .raw-foot {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.625rem;
  }

  .err {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--color-error, #dc2626);
  }
  .note {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
