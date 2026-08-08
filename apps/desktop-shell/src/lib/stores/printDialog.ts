/// The first-party print dialog (printing-plan.md PRN-R3): the print-a-document
/// moment. An app prints through `org.freedesktop.portal.Print`; the portal hands
/// the shell a pending request (title + which app), the shell shows THIS dialog to
/// choose the printer + options, and CUPS executes. The app never touches the
/// printer directly, only the result of a user-driven dialog - the same isolation
/// the file picker and the screencast source picker use.
///
/// Mock-vs-live: fixture-backed. The portal print backend (`PreparePrint` ->
/// `Print(fd)`), the request feed (`poll_print_request`), the submit/cancel path
/// (`submit_print` / `cancel_print`), printer enumeration in the shell
/// (`printers_list` / `printers_default`, which the settings backend already has),
/// the real first-page raster (`render_print_preview`), and the modal input-region
/// activation are coder seams. Under vite the store serves a fixture so the surface
/// renders and drives.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// How the paper maps to the sheet (matches the Printers panel vocabulary; the
/// coder maps to the portal/GTK `sides` strings at submit).
export type Duplex = "one-sided" | "two-sided-long" | "two-sided-short";
/// Colour mode (Printers-panel vocabulary; maps to `print-color-mode` at submit).
export type Color = "color" | "mono";
/// Paper size.
export type Paper = "a4" | "letter" | "legal";
/// Which pages to print.
export type RangeMode = "all" | "current" | "range";
/// Whether the destination is on this machine or across the network.
export type Destination = "local" | "network";

/// One choosable printer (mirrors the daemon `Printer` the Printers panel uses).
export interface Printer {
  name: string;
  uri: string;
  info: string | null;
  location: string | null;
  makeModel: string | null;
  state: "idle" | "processing" | "stopped" | "unknown";
  acceptingJobs: boolean;
  destination: Destination;
}

/// The pending document, from the portal request.
export interface PendingPrint {
  id: string;
  /// The document name, shown in the header.
  title: string;
  /// The requesting app (attested by the portal), shown as "from <app>".
  appName: string;
  appId: string;
  /// Page count, drives the range + the preview pager.
  pageCount: number;
}

/// What the dialog returns: the printer + the chosen options.
export interface PrintSettings {
  printer: string;
  copies: number;
  rangeMode: RangeMode;
  /// The custom range text (e.g. "1-5, 8"), meaningful when rangeMode is "range".
  rangeText: string;
  duplex: Duplex;
  color: Color;
  paper: Paper;
}

const FIXTURE_REQUEST: PendingPrint = {
  id: "print-fixture",
  title: "Quarterly report.pdf",
  appName: "Files",
  appId: "org.arlen.files",
  pageCount: 12,
};

const FIXTURE_PRINTERS: Printer[] = [
  { name: "Office HP", uri: "usb://HP/LaserJet", info: "HP LaserJet Pro", location: "Study", makeModel: "HP LaserJet Pro M404", state: "idle", acceptingJobs: true, destination: "local" },
  { name: "Brother", uri: "usb://Brother/HL", info: null, location: null, makeModel: "Brother HL-L2350DW", state: "idle", acceptingJobs: true, destination: "local" },
  { name: "Front desk", uri: "ipp://192.168.1.50/ipp/print", info: "Front desk MFP", location: "Reception", makeModel: "Canon imageRUNNER", state: "idle", acceptingJobs: true, destination: "network" },
];

/// The pending print request, or null when nothing is being printed.
export const current = writable<PendingPrint | null>(null);
/// The printers to choose from.
export const printers = writable<Printer[]>([]);
/// The CUPS default printer name, preselected.
export const defaultPrinter = writable<string | null>(null);
/// True while the printer list is the FIXTURE rather than this machine's real
/// printers - so a demo dialog never passes as a live one.
export const printersMocked = writable(false);

/// True when a real session could not read the printer list at all.
///
/// Separate from `printersMocked`, because the two say different things: mocked
/// means "these are examples", unavailable means "there is nothing here and that
/// is not a statement about your printers".
export const printersUnavailable = writable(false);

async function loadPrinters(): Promise<void> {
  try {
    const [list, def] = await Promise.all([
      invoke<Printer[]>("printers_list"),
      invoke<string | null>("printers_default"),
    ]);
    printers.set(list);
    defaultPrinter.set(def);
    printersMocked.set(false);
  } catch {
    if (import.meta.env.DEV) {
      // No backend under vite, so the fixture is what there is to design
      // against, labelled by `printersMocked`.
      printers.set(FIXTURE_PRINTERS);
      defaultPrinter.set("Office HP");
      printersMocked.set(true);
      printersUnavailable.set(false);
      return;
    }
    // A real session that could not enumerate printers. Offering "Office HP" as
    // the default here is not a labelling problem the badge can fix: the Print
    // button sends the job, and it has to go somewhere. No printers leaves the
    // submit disabled, which is the honest end state.
    printers.set([]);
    defaultPrinter.set(null);
    printersMocked.set(false);
    printersUnavailable.set(true);
  }
}

/// Open the dialog for the next pending print. Live: driven by the portal print
/// event / a poll of `poll_print_request`; under vite it serves the fixture. No
/// pending request (or no portal on an assembled boot) shows nothing.
export async function openPrintDialog(): Promise<void> {
  let req: PendingPrint | null = null;
  try {
    req = await invoke<PendingPrint | null>("poll_print_request");
    printersMocked.set(false);
  } catch {
    req = import.meta.env.DEV ? FIXTURE_REQUEST : null;
  }
  if (!req) return;
  current.set(req);
  await loadPrinters();
}

/// Send the job. Live: `submit_print` recalls the staged portal settings by the
/// request id and hands CUPS the document.
export async function submitPrint(settings: PrintSettings): Promise<void> {
  let id: string | null = null;
  current.subscribe((v) => (id = v?.id ?? null))();
  current.set(null);
  if (id === null) return;
  try {
    await invoke("submit_print", { id, settings });
  } catch {
    // No portal under vite: the optimistic close stands.
  }
}

/// Dismiss the dialog without printing (cancel is first-class). Live: resolve the
/// portal request as cancelled.
export async function cancelPrint(): Promise<void> {
  let id: string | null = null;
  current.subscribe((v) => (id = v?.id ?? null))();
  current.set(null);
  if (id === null) return;
  try {
    await invoke("cancel_print", { id });
  } catch {
    // mock
  }
}

/// The host behind a network printer's URI, for the print-as-egress honesty line;
/// null for local / mDNS-discovered queues.
export function hostOf(uri: string): string | null {
  const scheme = (uri.split(":")[0] ?? "").toLowerCase();
  if (scheme === "dnssd" || scheme === "mdns") return null;
  const after = uri.split("://")[1];
  if (!after) return null;
  const authority = after.split("/")[0] ?? "";
  if (!authority) return null;
  const [host] = authority.split(":");
  return host ? decodeURIComponent(host) : null;
}

/// A printer's display label (the Printers-panel rule).
export function displayName(p: Printer): string {
  return p.info ?? p.makeModel ?? p.name;
}
