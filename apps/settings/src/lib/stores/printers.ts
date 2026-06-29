/// The Printers panel store (printing-plan.md PRN-R4). Mirrors the print
/// daemon's model (`daemons/print/src/model.rs`) field-for-field and talks to
/// the intended Settings <-> print Tauri bridge (`printers_*` commands). That
/// bridge is the coder's lane and is not wired yet, so every call falls back to
/// a representative fixture: the panel is affordance-complete and goes live the
/// moment the commands land, with no frontend change. Clearly a mock until then.

import { writable, get } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";

/// IPP printer-state, mirrored from `model.rs::PrinterState`.
export type PrinterState = "idle" | "processing" | "stopped" | "unknown";

/// IPP job-state, mirrored from `model.rs::JobState`.
export type JobState =
  | "pending"
  | "held"
  | "processing"
  | "stopped"
  | "canceled"
  | "aborted"
  | "completed"
  | "unknown";

/// Local vs network, the print-as-egress classification (`model.rs::Destination`).
export type Destination = "local" | "network";

/// A configured printer queue (`model.rs::Printer`).
export interface Printer {
  name: string;
  uri: string;
  info: string | null;
  location: string | null;
  makeModel: string | null;
  state: PrinterState;
  acceptingJobs: boolean;
  destination: Destination;
}

/// A print job in a queue (`model.rs::Job`), plus the progress the queue view
/// shows when the daemon reports it.
export interface Job {
  id: number;
  printer: string;
  name: string | null;
  user: string | null;
  state: JobState;
  /// Pages done / total, when known (held/pending jobs have none).
  progress?: { done: number; total: number } | null;
}

/// A printer found by driverless discovery, not yet added.
export interface DiscoveredPrinter {
  name: string;
  uri: string;
  makeModel: string | null;
  destination: Destination;
  /// IPP-Everywhere / driverless (zero-config) vs needs a manual driver.
  driverless: boolean;
}

/// Per-printer default options the panel edits.
export interface PrinterOptions {
  duplex: "one-sided" | "two-sided-long" | "two-sided-short";
  color: "color" | "mono";
  paper: "a4" | "letter" | "legal";
}

export const DEFAULT_OPTIONS: PrinterOptions = {
  duplex: "one-sided",
  color: "color",
  paper: "a4",
};

interface PrintersState {
  printers: Printer[];
  /// The CUPS default printer's queue name, or null.
  defaultName: string | null;
  queue: Job[];
  discovered: DiscoveredPrinter[];
  options: Record<string, PrinterOptions>;
  loading: boolean;
  /// True while running on the fixture (the bridge isn't wired yet).
  mocked: boolean;
}

const initial: PrintersState = {
  printers: [],
  defaultName: null,
  queue: [],
  discovered: [],
  options: {},
  loading: false,
  mocked: false,
};

export const printers = writable<PrintersState>(initial);

/// The host portion of a printer URI, for the network-destination honesty line.
/// Service-discovery URIs (dnssd/mdns) carry a service instance name, not an
/// address worth showing, so they return null (the line just says "Network").
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

/// The transport word for the destination line ("USB", "Network", …).
export function transportOf(uri: string): string {
  const scheme = (uri.split(":")[0] ?? "").toLowerCase();
  const map: Record<string, string> = {
    usb: "USB",
    parallel: "Parallel",
    serial: "Serial",
    hp: "USB",
    socket: "Socket",
    ipp: "IPP",
    ipps: "IPPS",
    lpd: "LPD",
    dnssd: "Network",
    mdns: "Network",
  };
  return map[scheme] ?? "Network";
}

// ---- The fixture (the bridge isn't wired; see the file header). ------------

const FIXTURE: Pick<PrintersState, "printers" | "defaultName" | "queue" | "discovered"> = {
  defaultName: "Brother_HL_L2350DW",
  printers: [
    {
      name: "Brother_HL_L2350DW",
      uri: "usb://Brother/HL-L2350DW?serial=U63",
      info: "Brother HL-L2350DW",
      location: "Desk",
      makeModel: "Brother HL-L2350DW series",
      state: "idle",
      acceptingJobs: true,
      destination: "local",
    },
    {
      name: "Office_LaserJet",
      uri: "ipp://10.0.0.14:631/ipp/print",
      info: "Office LaserJet",
      location: "Hallway",
      makeModel: "HP LaserJet M428",
      state: "idle",
      acceptingJobs: true,
      destination: "network",
    },
    {
      name: "Reception",
      uri: "dnssd://Reception%20Printer._ipp._tcp.local/",
      info: "Reception",
      location: "Front desk",
      makeModel: "Canon imageRUNNER",
      state: "stopped",
      acceptingJobs: false,
      destination: "network",
    },
  ],
  queue: [
    {
      id: 412,
      printer: "Brother_HL_L2350DW",
      name: "report.pdf",
      user: "tim",
      state: "processing",
      progress: { done: 1, total: 3 },
    },
    { id: 413, printer: "Office_LaserJet", name: "invoice.pdf", user: "tim", state: "held", progress: null },
  ],
  discovered: [
    {
      name: "Canon PIXMA TS5350",
      uri: "usb://Canon/PIXMA-TS5350",
      makeModel: "Canon PIXMA TS5350",
      destination: "local",
      driverless: true,
    },
    {
      name: "HP OfficeJet Pro",
      uri: "ipp://10.0.0.21:631/ipp/print",
      makeModel: "HP OfficeJet Pro 9015",
      destination: "network",
      driverless: true,
    },
  ],
};

/// Load printers + queue + discovery. Tries the live bridge first; on any
/// failure (the common case until the bridge lands) it serves the fixture and
/// flags `mocked` so the UI can label itself honestly.
export async function load(): Promise<void> {
  printers.update((s) => ({ ...s, loading: true }));
  try {
    const [list, def, queue] = await Promise.all([
      invoke<Printer[]>("printers_list"),
      invoke<string | null>("printers_default"),
      invoke<Job[]>("print_queue"),
    ]);
    printers.update((s) => ({
      ...s,
      printers: list,
      defaultName: def,
      queue,
      loading: false,
      mocked: false,
    }));
  } catch {
    printers.update((s) => ({
      ...s,
      printers: FIXTURE.printers,
      defaultName: FIXTURE.defaultName,
      queue: FIXTURE.queue,
      discovered: FIXTURE.discovered,
      loading: false,
      mocked: true,
    }));
  }
}

/// Re-run driverless discovery (Avahi). Falls back to the fixture set.
export async function discover(): Promise<void> {
  try {
    const found = await invoke<DiscoveredPrinter[]>("printers_discover");
    printers.update((s) => ({ ...s, discovered: found }));
  } catch {
    printers.update((s) => ({ ...s, discovered: FIXTURE.discovered }));
  }
}

export async function setDefault(name: string): Promise<void> {
  try {
    await invoke("printers_set_default", { name });
  } catch {
    // optimistic in the mock
  }
  printers.update((s) => ({ ...s, defaultName: name }));
}

export async function setOptions(name: string, options: PrinterOptions): Promise<void> {
  printers.update((s) => ({ ...s, options: { ...s.options, [name]: options } }));
  try {
    await invoke("printers_set_options", { name, options });
  } catch {
    // optimistic in the mock
  }
}

export function optionsFor(name: string): PrinterOptions {
  return get(printers).options[name] ?? DEFAULT_OPTIONS;
}

export async function removePrinter(name: string): Promise<void> {
  try {
    await invoke("printers_remove", { name });
  } catch {
    // optimistic in the mock
  }
  printers.update((s) => {
    const next = s.printers.filter((p) => p.name !== name);
    return {
      ...s,
      printers: next,
      defaultName: s.defaultName === name ? (next[0]?.name ?? null) : s.defaultName,
    };
  });
}

export async function addPrinter(d: DiscoveredPrinter): Promise<void> {
  try {
    await invoke("printers_add", { uri: d.uri, name: d.name });
  } catch {
    // optimistic in the mock
  }
  printers.update((s) => ({
    ...s,
    discovered: s.discovered.filter((x) => x.uri !== d.uri),
    printers: [
      ...s.printers,
      {
        name: d.name,
        uri: d.uri,
        info: d.name,
        location: null,
        makeModel: d.makeModel,
        state: "idle",
        acceptingJobs: true,
        destination: d.destination,
      },
    ],
  }));
}

/// Add a printer by a manual URI (the IP/PPD fallback path).
export async function addByUri(uri: string, name: string): Promise<void> {
  const destination: Destination = /^(usb|parallel|serial|hp|file):/i.test(uri) ? "local" : "network";
  await addPrinter({ name, uri, makeModel: null, destination, driverless: false });
}

export async function cancelJob(id: number): Promise<void> {
  try {
    await invoke("print_job_cancel", { id });
  } catch {
    // optimistic
  }
  printers.update((s) => ({
    ...s,
    queue: s.queue.map((j) => (j.id === id ? { ...j, state: "canceled" } : j)),
  }));
}

export async function retryJob(id: number): Promise<void> {
  try {
    await invoke("print_job_retry", { id });
  } catch {
    // optimistic
  }
  printers.update((s) => ({
    ...s,
    queue: s.queue.map((j) => (j.id === id ? { ...j, state: "pending" } : j)),
  }));
}

export async function clearCompleted(): Promise<void> {
  printers.update((s) => ({
    ...s,
    queue: s.queue.filter(
      (j) => !["completed", "canceled", "aborted"].includes(j.state),
    ),
  }));
}

export async function testPage(name: string): Promise<void> {
  try {
    await invoke("printers_test_page", { name });
  } catch {
    // a test page is a no-op in the mock
  }
}
