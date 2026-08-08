<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The first-party print dialog (printing-plan.md PRN-R3): the print-a-document
  /// moment, portal-mediated so the app never touches the printer directly. Mounted
  /// once in the shell layout beside the other request dialogs; a preview on the
  /// left, the printer + options on the right. Fixture-backed under vite; the portal
  /// submit path + the real page raster are coder seams.
  import * as Dialog from "$lib/components/ui/dialog";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { NumberInput } from "@arlen/ui-kit/components/ui/number-input";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { FileText, Printer as PrinterIcon, ChevronLeft, ChevronRight } from "lucide-svelte";
  import {
    current,
    printers,
    defaultPrinter,
    printersMocked,
    printersUnavailable,
    submitFailed,
    submitPrint,
    cancelPrint,
    hostOf,
    displayName,
    type Duplex,
    type Color,
    type Paper,
    type RangeMode,
  } from "$lib/stores/printDialog";

  // The option value sets mirror the Printers panel so the two surfaces speak one
  // vocabulary; the coder maps to the portal/GTK strings at submit.
  // `$derived`, not `const`: the translator is a store, so a constant would freeze
  // English at import and a locale switch would leave these controls behind.
  const DUPLEX_OPTIONS = $derived([
    { value: "one-sided", label: $t("sh.print.oneSided") },
    { value: "two-sided-long", label: $t("sh.print.twoSided") },
    { value: "two-sided-short", label: $t("sh.print.twoSidedFlip") },
  ]);
  const COLOR_OPTIONS = $derived([
    { value: "color", label: $t("sh.print.colorColour") },
    { value: "mono", label: $t("sh.print.colorMono") },
  ]);
  const PAPER_OPTIONS = [
    { value: "a4", label: "A4" },
    { value: "letter", label: "Letter" },
    { value: "legal", label: "Legal" },
  ];
  const RANGE_OPTIONS = $derived([
    { value: "all", label: $t("sh.print.rangeAll") },
    { value: "current", label: $t("sh.print.rangeCurrent") },
    { value: "range", label: $t("sh.print.rangeRange") },
  ]);

  let printer = $state("");
  let copies = $state(1);
  let rangeMode = $state<RangeMode>("all");
  let rangeText = $state("");
  let duplex = $state<Duplex>("one-sided");
  let color = $state<Color>("color");
  let paper = $state<Paper>("a4");
  let page = $state(1);

  // Preselect the CUPS default (or the first) once the printer list loads.
  $effect(() => {
    if (!printer && $printers.length > 0) {
      printer = $defaultPrinter ?? $printers[0].name;
    }
  });

  const printerOptions = $derived($printers.map((p) => ({ value: p.name, label: displayName(p) })));
  const selected = $derived($printers.find((p) => p.name === printer) ?? null);
  // Print-as-egress honesty: a network printer means data leaving the machine, so
  // it is named. Local/USB shows nothing loud. Nothing is blocked.
  const netHost = $derived(selected?.destination === "network" ? (hostOf(selected.uri) ?? "remote host") : null);
  const sheetRatio = $derived(paper === "a4" ? "1 / 1.414" : paper === "legal" ? "1 / 1.647" : "1 / 1.294");

  function doPrint() {
    if (!printer) return;
    void submitPrint({ printer, copies, rangeMode, rangeText, duplex, color, paper });
  }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && $current) {
      e.preventDefault();
      void cancelPrint();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

{#if $current}
  {@const req = $current}
  <Dialog.Root open={true} onOpenChange={(o) => { if (!o) cancelPrint(); }}>
    <Dialog.Content class="max-w-2xl p-0">
      <div class="pd">
        <header class="pd-head">
          <PrinterIcon size={18} strokeWidth={2} aria-hidden="true" />
          <div class="pd-head-text">
            <h2 class="pd-title">{$t("sh.print.title")}</h2>
            <p class="pd-sub">{$t("sh.print.subtitle", { doc: req.title, app: req.appName })}</p>
          </div>
        </header>

        <div class="pd-body">
          <div class="pd-preview">
            <!-- A sheet of paper is white in every theme, so the sheet + its grey
                 placeholder marks are literal, not theme tokens. The real first-page
                 raster replaces this glyph (render_print_preview). -->
            <div class="pd-sheet" class:mono={color === "mono"} style={`aspect-ratio:${sheetRatio}`}>
              <FileText size={34} strokeWidth={1.5} aria-hidden="true" />
              <span class="pd-sheet-label">{$t("sh.print.preview")}</span>
            </div>
            {#if req.pageCount > 1}
              <div class="pd-pager">
                <button type="button" class="pd-page-btn" disabled={page <= 1} onclick={() => (page = Math.max(1, page - 1))} aria-label={$t("sh.print.prevPage")}>
                  <ChevronLeft size={15} strokeWidth={2} />
                </button>
                <span class="pd-page-num">{$t("sh.print.pageOf", { page, total: req.pageCount })}</span>
                <button type="button" class="pd-page-btn" disabled={page >= req.pageCount} onclick={() => (page = Math.min(req.pageCount, page + 1))} aria-label={$t("sh.print.nextPage")}>
                  <ChevronRight size={15} strokeWidth={2} />
                </button>
              </div>
            {/if}
          </div>

          <div class="pd-controls">
            {#if $printersMocked}
              <p class="pd-note">{$t("sh.print.mocked")}</p>
            {:else if $printersUnavailable}
              <p class="pd-note">{$t("sh.print.unavailable")}</p>
            {/if}
            <!-- The dialog is still here because nothing was sent. -->
            {#if $submitFailed}
              <p class="pd-note" role="alert">{$t("sh.print.submitFailed")}</p>
            {/if}

            <div class="pd-field">
              <span class="pd-label">{$t("sh.print.printer")}</span>
              <PopoverSelect value={printer} options={printerOptions} width="100%" ariaLabel={$t("sh.print.printer")} onchange={(v) => (printer = v)} />
              {#if netHost}
                <span class="pd-dest">{$t("sh.print.networkHost", { host: netHost })}</span>
              {/if}
            </div>

            <div class="pd-field">
              <span class="pd-label">{$t("sh.print.copies")}</span>
              <NumberInput value={copies} min={1} max={999} ariaLabel={$t("sh.print.copies")} onchange={(v) => (copies = v)} />
            </div>

            <div class="pd-field">
              <span class="pd-label">{$t("sh.print.range")}</span>
              <SegmentedControl value={rangeMode} options={RANGE_OPTIONS} ariaLabel={$t("sh.print.range")} onchange={(v) => (rangeMode = v as RangeMode)} />
              {#if rangeMode === "range"}
                <Input value={rangeText} placeholder={$t("sh.print.rangeHint")} aria-label={$t("sh.print.pageRange")} oninput={(e) => (rangeText = e.currentTarget.value)} />
              {/if}
            </div>

            <div class="pd-field">
              <span class="pd-label">{$t("sh.print.sides")}</span>
              <SegmentedControl value={duplex} options={DUPLEX_OPTIONS} ariaLabel={$t("sh.print.sides")} onchange={(v) => (duplex = v as Duplex)} />
            </div>

            <div class="pd-field">
              <span class="pd-label">{$t("sh.print.colour")}</span>
              <SegmentedControl value={color} options={COLOR_OPTIONS} ariaLabel={$t("sh.print.colour")} onchange={(v) => (color = v as Color)} />
            </div>

            <div class="pd-field">
              <span class="pd-label">{$t("sh.print.paper")}</span>
              <PopoverSelect value={paper} options={PAPER_OPTIONS} width="9rem" ariaLabel={$t("sh.print.paperSize")} onchange={(v) => (paper = v as Paper)} />
            </div>
          </div>
        </div>

        <footer class="pd-foot">
          <Button variant="outline" onclick={() => cancelPrint()}>{$t("sh.print.cancel")}</Button>
          <span class="pd-spacer"></span>
          <Button onclick={doPrint} disabled={!printer}>{$t("sh.print.submit")}</Button>
        </footer>
      </div>
    </Dialog.Content>
  </Dialog.Root>
{/if}

<style>
  .pd {
    display: flex;
    flex-direction: column;
    max-height: min(88vh, 640px);
  }
  .pd-head {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    padding: 1.15rem 1.25rem 0.9rem;
    color: var(--foreground);
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .pd-head-text {
    min-width: 0;
  }
  .pd-title {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--foreground);
  }
  .pd-sub {
    margin: 0.1rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pd-body {
    display: flex;
    gap: 1.5rem;
    padding: 1.15rem 1.25rem;
    flex-wrap: wrap;
  }

  .pd-preview {
    flex: 0 0 40%;
    min-width: 10rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.75rem;
  }
  /* A physical sheet is white in light and dark themes alike, so these are
     deliberate literals rather than palette tokens. */
  .pd-sheet {
    width: 100%;
    max-width: 13rem;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 0.4rem;
    background: #ffffff;
    color: var(--color-accent);
    border: 1px solid color-mix(in srgb, #000 12%, transparent);
    border-radius: var(--radius-input);
    box-shadow: 0 6px 18px color-mix(in srgb, #000 18%, transparent);
  }
  .pd-sheet.mono {
    color: #9ca3af;
  }
  .pd-sheet-label {
    font-size: var(--text-2xs);
    font-weight: 500;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #6b7280;
  }
  .pd-pager {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
  }
  .pd-page-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background: transparent;
    color: var(--foreground);
    cursor: pointer;
  }
  .pd-page-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .pd-page-btn:not(:disabled):hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .pd-page-num {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    font-variant-numeric: tabular-nums;
  }

  .pd-controls {
    flex: 1;
    min-width: 13rem;
    display: flex;
    flex-direction: column;
    gap: 0.85rem;
  }
  .pd-note {
    margin: 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .pd-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    align-items: flex-start;
  }
  .pd-label {
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 62%, transparent);
  }
  .pd-dest {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-variant-numeric: tabular-nums;
  }

  .pd-foot {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.85rem 1.25rem 1.15rem;
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .pd-spacer {
    flex: 1;
  }
</style>
