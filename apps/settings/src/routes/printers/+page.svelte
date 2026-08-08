<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Printers panel (printing-plan.md PRN-R4): the printer list, the print
  /// queue, and add-a-printer, on the settings-panel archetype - built on the
  /// kit `Row` (leading status column, right-aligned control cluster, full-width
  /// `below` for the options) exactly like the AI Providers panel, so it shares
  /// their alignment + rhythm. The Arlen angle is the print-as-egress honesty:
  /// network printing sends the document over the LAN, stated once for the
  /// section and carried per row by the "Network" label (§4.2). Reads/writes the
  /// print daemon through the `printers_*` Tauri bridge; until that lands the
  /// panel runs on a fixture (a banner says so).
  import { onMount } from "svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { Trash2, SlidersHorizontal, RefreshCw, Plus } from "lucide-svelte";

  import {
    printers,
    load,
    discover,
    setDefault,
    setOptions,
    optionsFor,
    removePrinter,
    addPrinter,
    addByUri,
    cancelJob,
    retryJob,
    clearCompleted,
    testPage,
    hostOf,
    transportOf,
    type Printer,
    type Job,
    type DiscoveredPrinter,
    type PrinterState,
    type JobState,
    type PrinterOptions,
  } from "$lib/stores/printers";

  onMount(load);

  let expanded = $state<string | null>(null);
  let confirmRemove = $state<Printer | null>(null);
  let addOpen = $state(false);
  let addName = $state("");
  let addUri = $state("");

  // These are `$derived`, not `const`, on purpose: the translator is a store, so a
  // module-level constant captures English at import and a locale switch would leave
  // every dropdown behind. See `coder-reports.md` on the script-block blind spot.
  const PRINTER_STATE_LABEL: Record<PrinterState, string> = $derived({
    idle: $t("s.pr.stIdle"),
    processing: $t("s.pr.stPrinting"),
    stopped: $t("s.pr.stPaused"),
    unknown: $t("s.pr.stUnknown"),
  });
  const JOB_STATE_LABEL: Record<JobState, string> = $derived({
    pending: $t("s.pr.jbQueued"),
    held: $t("s.pr.jbHeld"),
    processing: $t("s.pr.stPrinting"),
    stopped: $t("s.pr.jbStopped"),
    canceled: $t("s.pr.jbCanceled"),
    aborted: $t("s.pr.jbFailed"),
    completed: $t("s.pr.jbDone"),
    unknown: $t("s.pr.stUnknown"),
  });
  const DUPLEX_OPTIONS = $derived([
    { value: "one-sided", label: $t("s.pr.oneSided") },
    { value: "two-sided-long", label: $t("s.pr.twoSided") },
    { value: "two-sided-short", label: $t("s.pr.twoSidedFlip") },
  ]);
  const COLOR_OPTIONS = $derived([
    { value: "color", label: $t("s.pr.colour") },
    { value: "mono", label: $t("s.pr.mono") },
  ]);
  const PAPER_OPTIONS = [
    { value: "a4", label: "A4" },
    { value: "letter", label: "Letter" },
    { value: "legal", label: "Legal" },
  ];

  function displayName(p: Printer): string {
    return p.info ?? p.makeModel ?? p.name;
  }
  function notReady(p: Printer): boolean {
    return p.state !== "idle";
  }
  /// The quiet meta line: transport (USB / Network · host), the state word only
  /// when it isn't the resting "Ready" (the dot already says ready), and the
  /// "Default" marker on exactly the default printer (the only per-row hint of
  /// which is default - the dropdown above is where you change it).
  function metaLine(p: Printer): string {
    const parts: string[] = [];
    if (p.destination === "local") parts.push(transportOf(p.uri));
    else {
      const host = hostOf(p.uri);
      parts.push(host ? `Network · ${host}` : "Network");
    }
    if (notReady(p)) parts.push(PRINTER_STATE_LABEL[p.state]);
    if ($printers.defaultName === p.name) parts.push("Default");
    return parts.join(" · ");
  }

  /// The printer options for the "Default printer" selector.
  const defaultOptions = $derived(
    $printers.printers.map((p) => ({ value: p.name, label: displayName(p) })),
  );
  function jobPrinter(queueName: string): string {
    const p = $printers.printers.find((x) => x.name === queueName);
    return p ? displayName(p) : queueName;
  }
  function jobStateText(job: Job): string {
    if (job.state === "processing" && job.progress) {
      return `${JOB_STATE_LABEL.processing} ${job.progress.done}/${job.progress.total}`;
    }
    return JOB_STATE_LABEL[job.state];
  }
  function commitOptions(name: string, patch: Partial<PrinterOptions>) {
    setOptions(name, { ...optionsFor(name), ...patch });
  }
  async function submitManual() {
    const uri = addUri.trim();
    const name = addName.trim() || uri;
    if (!uri) return;
    await addByUri(uri, name);
    addUri = "";
    addName = "";
    addOpen = false;
  }
</script>

<Page title={$t("s.pr.title")} description={$t("s.pr.desc")}>
  <SectionGrid>
    {#if $printers.mocked}
      <p class="note">
        {$t("s.pr.mocked")}
      </p>
    {:else if $printers.unavailable}
      <p class="note">
        {$t("s.pr.unavailable")}
      </p>
    {/if}

    <Section label={$t("s.pr.printers")}>
      {#if $printers.printers.length === 0}
        <p class="empty">{$printers.unavailable ? $t("s.pr.noneUnknown") : $t("s.pr.none")}</p>
      {:else}
        <Row label={$t("s.pr.defaultPrinter")} description={$t("s.pr.defaultPrinterDesc")}>
          {#snippet control()}
            <PopoverSelect
              value={$printers.defaultName ?? ""}
              options={defaultOptions}
              placeholder={$t("s.pr.noneOption")}
              ariaLabel={$t("s.pr.defaultPrinter")}
              onchange={setDefault}
            />
          {/snippet}
        </Row>
      {/if}
      {#each $printers.printers as p (p.name)}
        {@render printerRow(p)}
      {/each}
    </Section>

    <Section label={$t("s.pr.queue")}>
      {#if $printers.queue.length === 0}
        <p class="empty">{$printers.unavailable ? $t("s.pr.queueUnknown") : $t("s.pr.queueEmpty")}</p>
      {:else}
        {#each $printers.queue as job (job.id)}
          {@render jobRow(job)}
        {/each}
        <div class="foot">
          <Button variant="ghost" size="sm" onclick={clearCompleted}>{$t("s.pr.clearFinished")}</Button>
        </div>
      {/if}
    </Section>

    <Section label={$t("s.pr.addPrinter")}>
      {#each $printers.discovered as d (d.uri)}
        {@render discoveredRow(d)}
      {/each}
      {#if $printers.discovered.length === 0}
        <!-- "found none" and "could not look" are the same empty list on screen
             and different facts about the network. -->
        <p class="empty">
          {#if $printers.discoverFailed || $printers.unavailable}
            {$t("s.pr.discoverFailed")}
          {:else}
            {$t("s.pr.noneDiscovered")}
          {/if}
        </p>
      {/if}
      <div class="foot">
        <Button variant="ghost" size="sm" onclick={discover}>
          <RefreshCw />
          {$t("s.pr.rescan")}
        </Button>
        <Button variant="ghost" size="sm" onclick={() => (addOpen = !addOpen)}>
          <Plus />
          {$t("s.pr.addByUri")}
        </Button>
      </div>
      {#if addOpen}
        <div class="manual">
          <Input placeholder={$t("s.pr.namePlaceholder")} bind:value={addName} aria-label={$t("s.pr.printerName")} />
          <Input placeholder={$t("s.pr.uriPlaceholder")} bind:value={addUri} aria-label={$t("s.pr.printerAddress")} />
          <Button variant="default" size="sm" disabled={!addUri.trim()} onclick={submitManual}>{$t("s.pr.add")}</Button>
        </div>
      {/if}
    </Section>
  </SectionGrid>
</Page>

{#snippet printerRow(p: Printer)}
  {@const opts = optionsFor(p.name)}
  <Row label={displayName(p)} description={metaLine(p)}>
    {#snippet leading()}
      <span class="dot" data-state={p.state} aria-hidden="true"></span>
    {/snippet}
    {#snippet control()}
      <span class="ctl">
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={$t("s.pr.printOptions")}
          aria-expanded={expanded === p.name}
          onclick={() => (expanded = expanded === p.name ? null : p.name)}
        >
          <SlidersHorizontal />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label={$t("s.pr.removePrinter")} onclick={() => (confirmRemove = p)}>
          <Trash2 />
        </Button>
      </span>
    {/snippet}
    {#snippet below()}
      {#if expanded === p.name}
        <div class="options">
          <label class="opt">
            <span>{$t("s.pr.sides")}</span>
            <SegmentedControl
              ariaLabel={$t("s.pr.sides")}
              value={opts.duplex}
              options={DUPLEX_OPTIONS}
              onchange={(v) => commitOptions(p.name, { duplex: v as PrinterOptions["duplex"] })}
            />
          </label>
          <label class="opt">
            <span>{$t("s.pr.colourLabel")}</span>
            <SegmentedControl
              ariaLabel={$t("s.pr.colourLabel")}
              value={opts.color}
              options={COLOR_OPTIONS}
              onchange={(v) => commitOptions(p.name, { color: v as PrinterOptions["color"] })}
            />
          </label>
          <label class="opt">
            <span>{$t("s.pr.paper")}</span>
            <PopoverSelect
              ariaLabel={$t("s.pr.paperSize")}
              value={opts.paper}
              options={PAPER_OPTIONS}
              onchange={(v) => commitOptions(p.name, { paper: v as PrinterOptions["paper"] })}
            />
          </label>
          <div class="opt-actions">
            <Button variant="ghost" size="sm" onclick={() => testPage(p.name)}>{$t("s.pr.testPage")}</Button>
          </div>
        </div>
      {/if}
    {/snippet}
  </Row>
{/snippet}

{#snippet jobRow(job: Job)}
  <Row label={job.name ?? `Job ${job.id}`} description={jobPrinter(job.printer)}>
    {#snippet leading()}
      <span class="dot" data-state={job.state === "processing" ? "processing" : "queued"} aria-hidden="true"></span>
    {/snippet}
    {#snippet control()}
      <span class="ctl">
        <span class="job-state" data-state={job.state}>{jobStateText(job)}</span>
        {#if job.state === "processing" || job.state === "pending"}
          <Button variant="ghost" size="sm" onclick={() => cancelJob(job.printer, job.id)}>{$t("s.pr.cancel")}</Button>
        {:else if job.state === "held" || job.state === "stopped"}
          <Button variant="ghost" size="sm" onclick={() => retryJob(job.id)}>{$t("s.pr.resume")}</Button>
        {/if}
      </span>
    {/snippet}
  </Row>
{/snippet}

{#snippet discoveredRow(d: DiscoveredPrinter)}
  <Row
    label={d.makeModel ?? d.name}
    description={d.driverless
      ? $t("s.pr.discDriverless", { transport: d.destination === "network" ? $t("s.pr.network") : transportOf(d.uri) })
      : d.destination === "network" ? $t("s.pr.network") : transportOf(d.uri)}
  >
    {#snippet leading()}
      <span class="dot ghost" aria-hidden="true"></span>
    {/snippet}
    {#snippet control()}
      <Button variant="outline" size="sm" onclick={() => addPrinter(d)}>{$t("s.pr.add")}</Button>
    {/snippet}
  </Row>
{/snippet}

<ConfirmDialog
  open={confirmRemove !== null}
  title={$t("s.pr.confirmTitle")}
  message={$t("s.pr.confirmMsg", { name: confirmRemove ? displayName(confirmRemove) : "" })}
  confirmLabel={$t("s.pr.confirmLabel")}
  variant="destructive"
  onConfirm={async () => {
    if (confirmRemove) await removePrinter(confirmRemove.name);
    confirmRemove = null;
  }}
  onCancel={() => (confirmRemove = null)}
/>

<style>
  .note {
    margin: 0;
    padding: 8px 12px;
    font-size: var(--text-xs);
    color: var(--color-fg-secondary);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
  }
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: var(--text-sm);
    color: var(--color-fg-secondary);
  }

  /* The status dot is the row's leading column (Providers keeps its own dot
     too); a fixed box keeps every name aligned down the panel. */
  .dot {
    display: block;
    width: 8px;
    height: 8px;
    border-radius: var(--radius-chip);
    background: var(--color-fg-disabled);
  }
  .dot.ghost {
    background: none;
    border: 1.5px solid var(--color-border-strong, var(--color-border));
  }
  .dot[data-state="idle"] {
    background: var(--color-success, #10b981);
  }
  .dot[data-state="processing"] {
    background: var(--color-accent);
  }
  .dot[data-state="stopped"] {
    background: var(--color-warning, #f59e0b);
  }

  /* The right-aligned action cluster (Row centres it; we only set the gap). */
  .ctl {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .job-state {
    margin-inline-end: 6px;
    font-size: var(--text-xs);
    color: var(--color-fg-secondary);
    white-space: nowrap;
  }
  .job-state[data-state="processing"] {
    color: var(--color-accent);
  }
  .job-state[data-state="aborted"] {
    color: var(--color-error, #ef4444);
  }

  /* The options live in the row's full-width `below` slot. */
  .options {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 16px;
    padding-top: 4px;
  }
  .opt {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: var(--text-xs);
    color: var(--color-fg-secondary);
  }
  .opt-actions {
    margin-inline-start: auto;
  }

  .foot {
    display: flex;
    gap: 8px;
    padding: 0.5rem 1rem;
  }
  .manual {
    display: flex;
    gap: 8px;
    padding: 0 1rem 0.5rem;
  }
</style>
