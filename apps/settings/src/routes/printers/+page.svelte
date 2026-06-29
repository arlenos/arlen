<script lang="ts">
  /// Printers panel (printing-plan.md PRN-R4): the printer list, the print
  /// queue, and add-a-printer, on the settings-panel archetype. The Arlen angle
  /// is the print-as-egress honesty - every printer states plainly whether it is
  /// on this machine or a network destination the document leaves to (§4.2).
  /// Reads/writes the print daemon through the `printers_*` Tauri bridge; until
  /// that bridge lands the panel runs on a fixture (a banner says so).
  import { onMount } from "svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { Globe, HardDrive, Printer as PrinterIcon, Trash2, SlidersHorizontal, RefreshCw } from "lucide-svelte";

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
    DEFAULT_OPTIONS,
    type Printer,
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

  function stateLabel(p: Printer): string {
    if (p.state === "stopped" && !p.acceptingJobs) return "Paused";
    return PRINTER_STATE_LABEL[p.state];
  }

  /// The destination honesty line: local stays on the machine; network is a
  /// send over the LAN, named so the user knows the document leaves.
  function destinationLine(p: Printer): string {
    const state = stateLabel(p);
    if (p.destination === "local") {
      return `On this machine · ${transportOf(p.uri)} · ${state}`;
    }
    const host = hostOf(p.uri);
    return `Network${host ? ` · ${host}` : ""} · sends over your LAN · ${state}`;
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
      <p class="mock-note">
        Showing example data. Live printers appear once the print service is connected.
      </p>
    {/if}

    <Group label="Printers">
      {#if $printers.printers.length === 0}
        <p class="empty">No printers yet. Add one below.</p>
      {/if}
      {#each $printers.printers as p (p.name)}
        {@const isDefault = $printers.defaultName === p.name}
        {@const opts = optionsFor(p.name)}
        <div class="printer" class:open={expanded === p.name}>
          <div class="printer-head">
            <span class="dot" data-state={p.state} aria-hidden="true"></span>
            <div class="ident">
              <div class="name-row">
                <span class="name">{p.info ?? p.makeModel ?? p.name}</span>
                {#if isDefault}<Badge>Default</Badge>{/if}
              </div>
              <div class="dest">
                {#if p.destination === "network"}
                  <Globe size={12} strokeWidth={1.75} />
                {:else}
                  <HardDrive size={12} strokeWidth={1.75} />
                {/if}
                <span>{destinationLine(p)}</span>
              </div>
            </div>
            <div class="actions">
              {#if !isDefault}
                <Button variant="ghost" size="sm" onclick={() => setDefault(p.name)}>Set default</Button>
              {/if}
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Print options"
                aria-expanded={expanded === p.name}
                onclick={() => (expanded = expanded === p.name ? null : p.name)}
              >
                <SlidersHorizontal />
              </Button>
              <Button variant="ghost" size="icon-sm" aria-label="Print a test page" onclick={() => testPage(p.name)}>
                <PrinterIcon />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Remove printer"
                onclick={() => (confirmRemove = p)}
              >
                <Trash2 />
              </Button>
            </div>
          </div>

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
            </div>
          {/if}
        </div>
      {/each}
    </Group>

    <Group label="Print queue">
      {#if $printers.queue.length === 0}
        <p class="empty">No jobs in the queue.</p>
      {:else}
        {#each $printers.queue as job (job.id)}
          <div class="job">
            <PrinterIcon size={14} strokeWidth={1.75} class="job-icon" />
            <span class="job-name">{job.name ?? `Job ${job.id}`}</span>
            <span class="job-printer">{job.printer}</span>
            <span class="job-state" data-state={job.state}>
              {JOB_STATE_LABEL[job.state]}{#if job.state === "processing" && job.progress}
                {" "}{job.progress.done}/{job.progress.total}{/if}
            </span>
            <div class="job-actions">
              {#if job.state === "processing" || job.state === "pending"}
                <Button variant="ghost" size="sm" onclick={() => cancelJob(job.id)}>Cancel</Button>
              {/if}
              {#if job.state === "held" || job.state === "stopped"}
                <Button variant="ghost" size="sm" onclick={() => retryJob(job.id)}>Resume</Button>
              {/if}
            </div>
          </div>
        {/each}
        <div class="queue-foot">
          <Button variant="ghost" size="sm" onclick={clearCompleted}>Clear finished</Button>
        </div>
      {/if}
    </Group>

    <Group label="Add a printer">
      {#if $printers.discovered.length > 0}
        <p class="discover-head">Discovered on your network and USB:</p>
        {#each $printers.discovered as d (d.uri)}
          <div class="discovered">
            {#if d.destination === "network"}
              <Globe size={14} strokeWidth={1.75} />
            {:else}
              <HardDrive size={14} strokeWidth={1.75} />
            {/if}
            <div class="disc-ident">
              <span class="disc-name">{d.makeModel ?? d.name}</span>
              <span class="disc-sub">
                {transportOf(d.uri)}{#if d.driverless} · driverless{/if}{#if d.destination === "network"} · network{/if}
              </span>
            </div>
            <Button variant="outline" size="sm" onclick={() => addPrinter(d)}>Add</Button>
          </div>
        {/each}
      {:else}
        <p class="empty">No printers discovered.</p>
      {/if}

      <div class="add-foot">
        <Button variant="ghost" size="sm" onclick={discover}>
          <RefreshCw />
          Rescan
        </Button>
        <Button variant="ghost" size="sm" onclick={() => (addOpen = !addOpen)}>Add by IP or URI</Button>
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

<ConfirmDialog
  open={confirmRemove !== null}
  title="Remove this printer?"
  message={`"${confirmRemove?.info ?? confirmRemove?.name ?? ""}" will be removed from this machine. Jobs in its queue are cancelled.`}
  confirmLabel="Remove"
  variant="destructive"
  onConfirm={async () => {
    if (confirmRemove) await removePrinter(confirmRemove.name);
    confirmRemove = null;
  }}
  onCancel={() => (confirmRemove = null)}
/>

<style>
  .mock-note {
    margin: 0;
    padding: 8px 12px;
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
  }
  .empty,
  .discover-head {
    margin: 0;
    padding: 6px 2px;
    font-size: 0.8125rem;
    color: var(--color-fg-secondary);
  }

  .printer {
    display: flex;
    flex-direction: column;
    padding: 10px 12px;
    border-radius: var(--radius-card);
    transition: background-color var(--duration-fast) var(--ease-out);
  }
  .printer.open {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .printer-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .dot {
    flex-shrink: 0;
    width: 9px;
    height: 9px;
    border-radius: var(--radius-full);
    background: var(--color-fg-disabled);
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
  .ident {
    flex: 1;
    min-width: 0;
  }
  .name-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--color-fg-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dest {
    display: flex;
    align-items: center;
    gap: 5px;
    margin-top: 2px;
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    white-space: nowrap;
  }
  .dest span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dest :global(svg) {
    flex-shrink: 0;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }

  .options {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    margin-top: 10px;
    padding-top: 10px;
    border-top: 1px solid var(--color-border);
  }
  .opt {
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
  }

  .job {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 2px;
  }
  .job :global(.job-icon) {
    flex-shrink: 0;
    color: var(--color-fg-secondary);
  }
  .job-name {
    font-size: 0.8125rem;
    color: var(--color-fg-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex: 1;
  }
  .job-printer {
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    flex-shrink: 0;
  }
  .job-state {
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    flex-shrink: 0;
    min-width: 64px;
    text-align: right;
  }
  .job-state[data-state="processing"] {
    color: var(--color-accent);
  }
  .job-state[data-state="aborted"] {
    color: var(--color-error, #ef4444);
  }
  .job-actions {
    flex-shrink: 0;
  }
  .queue-foot,
  .add-foot {
    display: flex;
    gap: 8px;
    padding-top: 6px;
  }

  .discovered {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 2px;
    color: var(--color-fg-secondary);
  }
  .disc-ident {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .disc-name {
    font-size: 0.8125rem;
    color: var(--color-fg-primary);
  }
  .disc-sub {
    font-size: 0.6875rem;
    color: var(--color-fg-secondary);
  }

  .manual {
    display: flex;
    gap: 8px;
    margin-top: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--color-border);
  }
</style>
