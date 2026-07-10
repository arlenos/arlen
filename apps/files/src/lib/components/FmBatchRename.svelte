<script lang="ts">
  /// The batch-rename dialog: a rule (find/replace, case, numbering) over the
  /// selected names with a live preview of old -> new and conflict flags. The
  /// preview is computed client-side (`bulk-rename.ts`, mirroring the core); the
  /// actual rename is applied by the host over the backend. Rides the shared
  /// modal shell.
  import { Dialog } from "@arlen/ui-kit/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import {
    planRename,
    type CaseTransform,
    type RenameRule,
  } from "$lib/bulk-rename";
  import { t } from "$lib/i18n/messages";

  type Props = {
    /// Whether the dialog is open.
    open: boolean;
    /// The selected filenames to rename, in order (drives the numbering).
    names: string[];
    /// Close without applying.
    onClose: () => void;
    /// Apply the rule (the host runs the rename over the backend).
    onApply: (rule: RenameRule) => void;
  };

  let { open, names, onClose, onApply }: Props = $props();

  let find = $state("");
  let replace = $state("");
  let ignoreCase = $state(false);
  let caseMode = $state("none");
  let numberingOn = $state(false);
  let pattern = $state("{name}-{n}");
  let start = $state(1);
  let step = $state(1);
  let pad = $state(2);

  const rule = $derived<RenameRule>({
    find: find.length > 0 ? find : undefined,
    replace,
    find_case_insensitive: ignoreCase,
    case: caseMode === "none" ? undefined : (caseMode as CaseTransform),
    numbering: numberingOn ? { template: pattern, start, step, pad } : undefined,
  });

  const preview = $derived(planRename(names, rule));
  const changing = $derived(
    preview.filter((p) => p.conflict === "none").length,
  );
  const skipped = $derived(
    preview.filter((p) => p.conflict === "duplicate" || p.conflict === "invalid")
      .length,
  );
</script>

<Dialog {open} {onClose} ariaLabel={$t("f.rename.aria")} size="lg">
  <div class="br">
    <h2 class="br-title">{$t("f.rename.title", { count: names.length })}</h2>

    <div class="br-rule">
      <div class="br-field br-grow">
        <span class="br-label">{$t("f.rename.find")}</span>
        <Input bind:value={find} placeholder={$t("f.rename.findPlaceholder")} />
      </div>
      <div class="br-field br-grow">
        <span class="br-label">{$t("f.rename.replaceWith")}</span>
        <Input bind:value={replace} placeholder={$t("f.rename.replacePlaceholder")} />
      </div>
      <label class="br-toggle">
        <Switch bind:value={ignoreCase} />
        <span>{$t("f.rename.ignoreCase")}</span>
      </label>
    </div>

    <div class="br-rule">
      <div class="br-field">
        <span class="br-label">{$t("f.rename.case")}</span>
        <SegmentedControl
          bind:value={caseMode}
          options={[
            { value: "none", label: $t("f.rename.caseNone") },
            { value: "lower", label: $t("f.rename.caseLower") },
            { value: "upper", label: $t("f.rename.caseUpper") },
            { value: "title", label: $t("f.rename.caseTitle") },
          ]}
        />
      </div>
      <label class="br-toggle br-toggle-end">
        <Switch bind:value={numberingOn} />
        <span>{$t("f.rename.addNumbering")}</span>
      </label>
    </div>

    {#if numberingOn}
      <div class="br-rule">
        <div class="br-field br-grow">
          <span class="br-label">{$t("f.rename.pattern")}</span>
          <Input bind:value={pattern} placeholder={"{name}-{n}"} />
        </div>
        <div class="br-field br-narrow">
          <span class="br-label">{$t("f.rename.start")}</span>
          <NumberInput value={start} min={0} onchange={(v) => (start = v)} />
        </div>
        <div class="br-field br-narrow">
          <span class="br-label">{$t("f.rename.step")}</span>
          <NumberInput value={step} min={1} onchange={(v) => (step = v)} />
        </div>
        <div class="br-field br-narrow">
          <span class="br-label">{$t("f.rename.digits")}</span>
          <NumberInput value={pad} min={1} max={6} onchange={(v) => (pad = v)} />
        </div>
      </div>
    {/if}

    <div class="br-preview">
      <ul class="br-list">
        {#each preview as row (row.old)}
          <li
            class="br-row"
            class:is-conflict={row.conflict === "duplicate" ||
              row.conflict === "invalid"}
          >
            <span class="br-old">{row.old}</span>
            <span class="br-arrow">&rarr;</span>
            <span class="br-new" class:is-unchanged={row.conflict === "unchanged"}>
              {row.new}
            </span>
            {#if row.conflict === "duplicate"}
              <span class="br-badge">{$t("f.rename.badgeDuplicate")}</span>
            {:else if row.conflict === "invalid"}
              <span class="br-badge">{$t("f.rename.badgeInvalid")}</span>
            {/if}
          </li>
        {/each}
      </ul>
    </div>

    <div class="br-foot">
      <span class="br-summary">
        {skipped > 0 ? $t("f.rename.willChangeSkipped", { changing, skipped }) : $t("f.rename.willChange", { changing })}
      </span>
      <span class="br-spacer"></span>
      <Button variant="ghost" onclick={onClose}>{$t("f.ops.cancel")}</Button>
      <Button variant="default" onclick={() => onApply(rule)}>
        {$t("f.rename.confirm", { changing })}
      </Button>
    </div>
  </div>
</Dialog>

<style>
  .br {
    display: flex;
    flex-direction: column;
    gap: 16px;
    padding: 20px;
  }
  .br-title {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .br-rule {
    display: flex;
    align-items: flex-end;
    gap: 12px;
  }
  .br-field {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }
  .br-grow {
    flex: 1;
  }
  .br-narrow {
    width: 72px;
    flex-shrink: 0;
  }
  .br-label {
    font-size: 0.6875rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .br-toggle {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    height: var(--height-control, 28px);
    font-size: 0.75rem;
    color: var(--foreground);
    white-space: nowrap;
  }
  .br-toggle-end {
    margin-inline-start: auto;
  }

  .br-preview {
    max-height: 280px;
    overflow-y: auto;
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--foreground) 2%, transparent);
  }
  .br-list {
    list-style: none;
    margin: 0;
    padding: 4px 0;
  }
  .br-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 12px;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
  }
  .br-old {
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 38%;
  }
  .br-arrow {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .br-new {
    flex: 1;
    min-width: 0;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .br-new.is-unchanged {
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  .br-row.is-conflict .br-new {
    color: var(--color-error, #c96a6a);
  }
  .br-badge {
    flex-shrink: 0;
    font-family: var(--font-sans, inherit);
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-error, #c96a6a);
  }

  .br-foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .br-summary {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .br-spacer {
    flex: 1;
  }
</style>
