<script lang="ts">
  /// Printers panel (printing-plan.md PRN-R4): the printer list, the print
  /// queue, and add-a-printer, on the settings-panel archetype. The Arlen angle
  /// is the print-as-egress honesty - network printing sends the document over
  /// the LAN, stated once for the section and carried per row by the "Network"
  /// label (§4.2). Reads/writes the print daemon through the `printers_*` Tauri
  /// bridge; until that bridge lands the panel runs on a fixture (a banner says
  /// so).
  import { onMount } from "svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { CircleCheck, Circle, Trash2, SlidersHorizontal, RefreshCw, Plus } from "lucide-svelte";

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

  const anyNetwork = $derived($printers.printers.some((p) => p.destination === "network"));

  /// The printer's display name, preferring the human description.
  function displayName(p: Printer): string {
    return p.info ?? p.makeModel ?? p.name;
  }

  function notReady(p: Printer): boolean {
    return p.state !== "idle";
  }

  /// The quiet meta line: the transport (USB / Network · host), and the state
  /// word ONLY when it isn't the resting "Ready" (the dot already says ready).
  function metaLine(p: Printer): string {
    const parts: string[] = [];
    if (p.destination === "local") {
      parts.push(transportOf(p.uri));
    } else {
      const host = hostOf(p.uri);
      parts.push(host ? `Network · ${host}` : "Network");
    }
    if (notReady(p)) parts.push(PRINTER_STATE_LABEL[p.state]);
    return parts.join(" · ");
  }

  /// Resolve a queue job's printer to its friendly name.
  function jobPrinter(queueName: string): string {
    const p = $printers.printers.find((x) => x.name === queueName);
    return p ? displayName(p) : queueName;
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
      {#if anyNetwork}
        <p class="note subtle">
          Printing to a network printer sends the document over your local network.
        </p>
      {/if}
      {#if $printers.printers.length === 0}
        <p class="empty">No printers yet. Add one below.</p>
      {/if}
      {#each $printers.printers as p (p.name)}
        {@const isDefault = $printers.defaultName === p.name}
        {@const opts = optionsFor(p.name)}
        <div class="printer" class:open={expanded === p.name}>
          <div class="row">
            <button
              class="def"
              class:active={isDefault}
              disabled={isDefault}
              aria-label={isDefault ? "Default printer" : `Make ${displayName(p)} the default`}
              title={isDefault ? "Default printer" : "Set as default"}
              onclick={() => setDefault(p.name)}
            >
              {#if isDefault}
                <CircleCheck size={17} strokeWidth={1.75} />
              {:else}
                <Circle size={17} strokeWidth={1.75} />
              {/if}
            </button>
            <span class="dot" data-state={p.state} aria-hidden="true"></span>
            <div class="ident">
              <span class="name">{displayName(p)}</span>
              <span class="meta">{metaLine(p)}</span>
            </div>
            <div class="actions">
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label="Print options"
                aria-expanded={expanded === p.name}
                onclick={() => (expanded = expanded === p.name ? null : p.name)}
              >
                <SlidersHorizontal />
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
              <div class="opt-actions">
                <Button variant="ghost" size="sm" onclick={() => testPage(p.name)}>Print test page</Button>
              </div>
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
          <div class="row job">
            <div class="ident">
              <span class="name">{job.name ?? `Job ${job.id}`}</span>
              <span class="meta">{jobPrinter(job.printer)}</span>
            </div>
            <span class="job-state" data-state={job.state}>
              {JOB_STATE_LABEL[job.state]}{#if job.state === "processing" && job.progress}
                {" "}{job.progress.done}/{job.progress.total}{/if}
            </span>
            <div class="actions">
              {#if job.state === "processing" || job.state === "pending"}
                <Button variant="ghost" size="sm" onclick={() => cancelJob(job.id)}>Cancel</Button>
              {:else if job.state === "held" || job.state === "stopped"}
                <Button variant="ghost" size="sm" onclick={() => retryJob(job.id)}>Resume</Button>
              {/if}
            </div>
          </div>
        {/each}
        <div class="foot">
          <Button variant="ghost" size="sm" onclick={clearCompleted}>Clear finished</Button>
        </div>
      {/if}
    </Group>

    <Group label="Add a printer">
      {#each $printers.discovered as d (d.uri)}
        <div class="row disc">
          <div class="ident">
            <span class="name">{d.makeModel ?? d.name}</span>
            <span class="meta">
              {d.destination === "network" ? "Network" : transportOf(d.uri)}{#if d.driverless} · driverless{/if}
            </span>
          </div>
          <div class="actions">
            <Button variant="outline" size="sm" onclick={() => addPrinter(d)}>Add</Button>
          </div>
        </div>
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
  .note.subtle {
    padding: 2px 2px 8px;
    background: none;
    border: none;
  }
  .empty {
    margin: 0;
    padding: 6px 2px;
    font-size: 0.8125rem;
    color: var(--color-fg-secondary);
  }

  .printer {
    border-radius: var(--radius-card);
    transition: background-color var(--duration-fast) var(--ease-out);
  }
  .printer.open {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }

  /* The printer row: a fixed default-control + status column, the identity,
     then the action cluster. Queue and discovery rows reuse the row rhythm and
     indent their identity to the same left edge (60px = 8 pad + 24 toggle + 10
     + 8 dot + 10) so every name lines up down the panel. */
  .row {
    display: grid;
    grid-template-columns: 24px 8px 1fr auto;
    align-items: center;
    gap: 10px;
    min-height: 44px;
    padding: 4px 8px;
  }
  .row.job {
    grid-template-columns: 1fr auto auto;
    padding-left: 60px;
  }
  .row.disc {
    grid-template-columns: 1fr auto;
    padding-left: 60px;
  }

  .def {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-fg-disabled);
    border-radius: var(--radius-full);
    transition: color var(--duration-fast) var(--ease-out);
  }
  .def:hover:not(:disabled) {
    color: var(--color-fg-secondary);
  }
  .def.active {
    color: var(--color-accent);
  }

  .dot {
    width: 8px;
    height: 8px;
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
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .name {
    font-size: 0.875rem;
    color: var(--color-fg-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 2px;
    justify-self: end;
  }

  .job-state {
    font-size: 0.75rem;
    color: var(--color-fg-secondary);
    white-space: nowrap;
    justify-self: end;
  }
  .job-state[data-state="processing"] {
    color: var(--color-accent);
  }
  .job-state[data-state="aborted"] {
    color: var(--color-error, #ef4444);
  }

  .options {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 16px;
    margin: 0 8px 4px;
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
  .opt-actions {
    margin-left: auto;
  }

  .foot {
    display: flex;
    gap: 8px;
    padding: 6px 4px 0;
  }

  .manual {
    display: flex;
    gap: 8px;
    margin: 8px 4px 0;
    padding-top: 10px;
    border-top: 1px solid var(--color-border);
  }
</style>
