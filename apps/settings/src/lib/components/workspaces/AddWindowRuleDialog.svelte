<script lang="ts">
  /// Modal dialog for adding a new window rule.
  ///
  /// V1 scope (Sprint B): regex matchers for `app_id` and `title`,
  /// plus a Float/Tile radio. Window-type matching ("dialog") is
  /// supported in the compositor but rare in user configs — we
  /// expose it as an optional checkbox.
  ///
  /// Validates regex patterns inline (`new RegExp(p)` try/catch)
  /// so the user can't ship an unparseable pattern that crashes
  /// the compositor's window-rule loop.

  import { Button } from "$lib/components/ui/button";
  import { Input } from "$lib/components/ui/input";
  import { Switch } from "$lib/components/ui/switch";
  import type { WindowRule, WindowRuleAction } from "$lib/stores/workspaces";

  let {
    open,
    onAdd,
    onCancel,
  }: {
    open: boolean;
    onAdd: (rule: WindowRule) => void;
    onCancel: () => void;
  } = $props();

  let appIdPattern = $state("");
  let titlePattern = $state("");
  let matchDialog = $state(false);
  let action = $state<WindowRuleAction>("float");
  let appIdError = $state<string | null>(null);
  let titleError = $state<string | null>(null);

  function validateRegex(pattern: string): string | null {
    if (pattern.length === 0) return null;
    try {
      new RegExp(pattern);
      return null;
    } catch (e) {
      return e instanceof Error ? e.message : "Invalid regex";
    }
  }

  $effect(() => {
    appIdError = validateRegex(appIdPattern);
  });
  $effect(() => {
    titleError = validateRegex(titlePattern);
  });

  const hasMatcher = $derived(
    appIdPattern.trim().length > 0 ||
      titlePattern.trim().length > 0 ||
      matchDialog,
  );

  const canSubmit = $derived(
    hasMatcher && appIdError === null && titleError === null,
  );

  function reset() {
    appIdPattern = "";
    titlePattern = "";
    matchDialog = false;
    action = "float";
    appIdError = null;
    titleError = null;
  }

  function submit() {
    if (!canSubmit) return;
    // Nested `match` table is what the compositor parser expects.
    // Flat-key rules are silently dropped at compositor load time
    // (see `parse_layout_config` in compositor/src/config/mod.rs).
    const rule: WindowRule = {
      match: {},
      action,
    };
    if (appIdPattern.trim()) rule.match.app_id = appIdPattern.trim();
    if (titlePattern.trim()) rule.match.title = titlePattern.trim();
    if (matchDialog) rule.match.window_type = "dialog";
    onAdd(rule);
    reset();
  }

  function cancel() {
    reset();
    onCancel();
  }
</script>

{#if open}
  <div
    class="backdrop"
    onclick={cancel}
    onkeydown={(e) => {
      if (e.key === "Escape") cancel();
    }}
    role="presentation"
  >
    <div
      class="dialog"
      role="dialog"
      aria-labelledby="window-rule-title"
      aria-modal="true"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
      onkeydown={(e) => e.stopPropagation()}
    >
      <h2 id="window-rule-title" class="title">Add Window Rule</h2>
      <p class="description">
        At least one matcher must be set. Patterns are regular
        expressions matched against the window's app_id / title.
      </p>

      <div class="field">
        <label for="rule-app-id">App ID pattern</label>
        <Input
          id="rule-app-id"
          bind:value={appIdPattern}
          placeholder="e.g. firefox|chromium"
        />
        {#if appIdError}<div class="field-error">{appIdError}</div>{/if}
      </div>

      <div class="field">
        <label for="rule-title">Title pattern</label>
        <Input
          id="rule-title"
          bind:value={titlePattern}
          placeholder="e.g. Preferences"
        />
        {#if titleError}<div class="field-error">{titleError}</div>{/if}
      </div>

      <div class="field-toggle">
        <label for="rule-dialog">
          <span>Match dialog windows</span>
          <span class="hint">Toolkit-declared dialog hint (xdg-decoration)</span>
        </label>
        <Switch
          value={matchDialog}
          ariaLabel="Match dialog windows"
          onchange={(v) => (matchDialog = v)}
        />
      </div>

      <div class="field">
        <span class="label-static">Action</span>
        <div class="radio-row">
          <label class="radio">
            <input
              type="radio"
              bind:group={action}
              value="float"
              name="action"
            />
            <span>Float</span>
          </label>
          <label class="radio">
            <input
              type="radio"
              bind:group={action}
              value="tile"
              name="action"
            />
            <span>Tile</span>
          </label>
        </div>
      </div>

      <div class="footer">
        <Button variant="ghost" onclick={cancel}>Cancel</Button>
        <Button disabled={!canSubmit} onclick={submit}>Add Rule</Button>
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .dialog {
    width: 100%;
    max-width: 420px;
    background: var(--card);
    color: var(--foreground);
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    border-radius: var(--radius-input);
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
    box-shadow:
      0 24px 64px rgba(0, 0, 0, 0.4),
      0 8px 16px rgba(0, 0, 0, 0.3);
  }

  .title {
    font-size: 1rem;
    font-weight: 600;
    margin: 0;
  }

  .description {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    margin: 0;
    line-height: 1.4;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
  }

  .field label,
  .label-static {
    font-size: 0.8125rem;
    font-weight: 500;
  }

  .field-error {
    font-size: 0.75rem;
    color: var(--destructive);
  }

  .field-toggle {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.875rem;
  }

  .field-toggle label {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .hint {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    font-weight: 400;
  }

  .radio-row {
    display: flex;
    gap: 1rem;
  }

  .radio {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    font-size: 0.8125rem;
    cursor: pointer;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.25rem;
  }
</style>
