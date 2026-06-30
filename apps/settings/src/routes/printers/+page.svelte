<script lang="ts">
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
  import { Group } from "@arlen/ui-kit/components/ui/group";
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

  const PRINTER_STATE_LABEL: Record<PrinterState, string> = {
    idle: "Ready",
    processing: "Printing",
    stopped: "Paused",
    unknown: "Unknown",
  };
  const JOB_STATE_LABEL: Record<JobState, string> = {
    pending: "Queued",
    held: "Held",
    processing: "Printing",
    stopped: "Stopped",
    canceled: "Canceled",
    aborted: "Failed",
    completed: "Done",
    unknown: "Unknown",
  };
  const DUPLEX_OPTIONS = [
    { value: "one-sided", label: "One-sided" },
    { value: "two-sided-long", label: "Two-sided" },
    { value: "two-sided-short", label: "Two-sided (flip)" },
  ];
  const COLOR_OPTIONS = [
    { value: "color", label: "Colour" },
    { value: "mono", label: "Black & white" },
  ];
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

<Page title="Printers" description="Manage printers, the print queue, and add new devices.">
  <SectionGrid>
    {#if $printers.mocked}
      <p class="note">
        Showing example data. Live printers appear once the print service is connected.
      </p>
    {/if}

    <Group label="Printers">
      {#if $printers.printers.length === 0}
        <p class="empty">No printers yet. Add one below.</p>
      {:else}
        <Row label="Default printer" description="Used unless an app picks another.">
          {#snippet control()}
            <PopoverSelect
              value={$printers.defaultName ?? ""}
              options={defaultOptions}
              placeholder="None"
              ariaLabel="Default printer"
              width="200px"
              onchange={setDefault}
            />
          {/snippet}
        </Row>
      {/if}
      {#each $printers.printers as p (p.name)}
        {@render printerRow(p)}
      {/each}
    </Group>

    <Group label="Print queue">
      {#if $printers.queue.length === 0}
        <p class="empty">No jobs in the queue.</p>
      {:else}
        {#each $printers.queue as job (job.id)}
          {@render jobRow(job)}
        {/each}
        <div class="foot">
          <Button variant="ghost" size="sm" onclick={clearCompleted}>Clear finished</Button>
        </div>
      {/if}
    </Group>

    <Group label="Add a printer">
      {#each $printers.discovered as d (d.uri)}
        {@render discoveredRow(d)}
      {/each}
      {#if $printers.discovered.length === 0}
        <p class="empty">No printers discovered.</p>
      {/if}
      <div class="foot">
        <Button variant="ghost" size="sm" onclick={discover}>
          <RefreshCw />
          Rescan
        </Button>
        <Button variant="ghost" size="sm" onclick={() => (addOpen = !addOpen)}>
          <Plus />
          Add by IP or URI
        </Button>
      </div>
      {#if addOpen}
        <div class="manual">
          <Input placeholder="Name (optional)" bind:value={addName} aria-label="Printer name" />
          <Input placeholder="ipp://10.0.0.20/ipp/print" bind:value={addUri} aria-label="Printer address" />
          <Button variant="default" size="sm" disabled={!addUri.trim()} onclick={submitManual}>Add</Button>
        </div>
      {/if}
    </Group>
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
          aria-label="Print options"
          aria-expanded={expanded === p.name}
          onclick={() => (expanded = expanded === p.name ? null : p.name)}
        >
          <SlidersHorizontal />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Remove printer" onclick={() => (confirmRemove = p)}>
          <Trash2 />
        </Button>
      </span>
    {/snippet}
    {#snippet below()}
      {#if expanded === p.name}
        <div class="options">
          <label class="opt">
            <span>Sides</span>
            <SegmentedControl
              ariaLabel="Sides"
              value={opts.duplex}
              options={DUPLEX_OPTIONS}
              onchange={(v) => commitOptions(p.name, { duplex: v as PrinterOptions["duplex"] })}
            />
          </label>
          <label class="opt">
            <span>Colour</span>
            <SegmentedControl
              ariaLabel="Colour"
              value={opts.color}
              options={COLOR_OPTIONS}
              onchange={(v) => commitOptions(p.name, { color: v as PrinterOptions["color"] })}
            />
          </label>
          <label class="opt">
            <span>Paper</span>
            <PopoverSelect
              ariaLabel="Paper size"
              value={opts.paper}
              options={PAPER_OPTIONS}
              width="140px"
              onchange={(v) => commitOptions(p.name, { paper: v as PrinterOptions["paper"] })}
            />
          </label>
          <div class="opt-actions">
            <Button variant="ghost" size="sm" onclick={() => testPage(p.name)}>Print test page</Button>
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
          <Button variant="ghost" size="sm" onclick={() => cancelJob(job.id)}>Cancel</Button>
        {:else if job.state === "held" || job.state === "stopped"}
          <Button variant="ghost" size="sm" onclick={() => retryJob(job.id)}>Resume</Button>
        {/if}
      </span>
    {/snippet}
  </Row>
{/snippet}

{#snippet discoveredRow(d: DiscoveredPrinter)}
  <Row
    label={d.makeModel ?? d.name}
    description={`${d.destination === "network" ? "Network" : transportOf(d.uri)}${d.driverless ? " · driverless" : ""}`}
  >
    {#snippet leading()}
      <span class="dot ghost" aria-hidden="true"></span>
    {/snippet}
    {#snippet control()}
      <Button variant="outline" size="sm" onclick={() => addPrinter(d)}>Add</Button>
    {/snippet}
  </Row>
{/snippet}

<ConfirmDialog
  open={confirmRemove !== null}
  title="Remove this printer?"
  message={`"${confirmRemove ? displayName(confirmRemove) : ""}" will be removed from this machine. Jobs in its queue are cancelled.`}
  confirmLabel="Remove"
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
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
  }
  .empty {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem;
    font-size: 0.8125rem;
    color: var(--color-fg-secondary);
  }

  /* The status dot is the row's leading column (Providers keeps its own dot
     too); a fixed box keeps every name aligned down the panel. */
  .dot {
    display: block;
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
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
    margin-right: 6px;
    font-size: 0.75rem;
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
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
  }
  .opt-actions {
    margin-left: auto;
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
