/// Local AI models for the Model Manager surface (local-model-bundle-plan.md).
/// Curated + non-technical: a plain-language fit verdict computed locally
/// (never a download to find out), quant hidden (the backend silently picks the
/// best-fitting Q4 ladder), size in GB, download progress + cancel, delete to
/// reclaim space, one active-model switch. Avoids the HF firehose, quant jargon,
/// and VRAM-as-primary; on an APU the speed word leads.
///
/// The `ai-model-manager` crate already computes fit/speed/quant/tier-picks and
/// does the SSRF-pinned sha256 download; only the Settings Tauri bridge is
/// unwired, so this reads a fixture and simulates progress until it lands.

import { invoke } from "@tauri-apps/api/core";
import { writable, get } from "svelte/store";

/// A recommendation tier: three clear choices, hardware-picked.
export type Tier = "fast" | "balanced" | "quality";

/// The plain 3-way fit verdict from `ai-model-manager::fit_badge`.
export type Fit = "fits" | "may-be-slow" | "wont-fit";

/// One curated model as the surface shows it (backend `CuratedModel` +
/// `Recommendation`, quant already resolved to the best-fitting one).
export interface LocalModel {
  /// The GGUF source repo, e.g. "bartowski/Llama-3.2-1B-Instruct-GGUF".
  source: string;
  name: string;
  paramsB: number;
  /// Plain task tags ("general", "coding", "writing").
  tasks: string[];
  advanced: boolean;
  fit: Fit;
  /// Estimated speed on this machine (tokens per second).
  tokensPerSec: number;
  /// Download size in GB at the silently-chosen quant.
  sizeGb: number;
  installed: boolean;
  active: boolean;
  /// The sub-1 GB model baked into the image (offline first boot, zero egress).
  baked: boolean;
}

/// The machine's capability, in plain words (backend `Hardware` +
/// `detect_hardware`), phrased for a non-technical reader.
export interface Hardware {
  ramGb: number;
  accelerator: "apu" | "discrete";
  vramGb: number | null;
  /// "Your machine can run models up to about 8B smoothly."
  summary: string;
}

/// An in-flight download (backend streams bytes; this mirrors the progress).
export interface Download {
  source: string;
  bytesFetched: number;
  totalBytes: number;
  status: "downloading" | "verifying" | "complete" | "error";
}

const TASK_LABELS: Record<string, string> = {
  general: "Everyday",
  coding: "Coding",
  writing: "Writing",
  reasoning: "Reasoning",
};

/// A plain label for a task tag.
export function taskLabel(task: string): string {
  return TASK_LABELS[task] ?? task;
}

const TIER_LABELS: Record<Tier, { label: string; note: string }> = {
  fast: { label: "Fast", note: "Snappy on your machine, lighter answers." },
  balanced: { label: "Balanced", note: "The sweet spot for most people." },
  quality: { label: "Best quality", note: "The strongest that still runs well." },
};

/// The label and one-line note for a tier.
export function tierMeta(tier: Tier): { label: string; note: string } {
  return TIER_LABELS[tier];
}

/// The models, the hardware line, and any active download.
export const models = writable<LocalModel[]>([]);
export const hardware = writable<Hardware | null>(null);
export const download = writable<Download | null>(null);
export const modelsLoaded = writable(false);

// Mock hardware: a 7840U-class APU (Tim's datapoint) - 61 GB RAM, integrated
// GPU, ~9.4 tok/s on a 7B-Q4. Leads with the speed/size story, not VRAM.
const MOCK_HARDWARE: Hardware = {
  ramGb: 61,
  accelerator: "apu",
  vramGb: null,
  summary: "Your machine can run models up to about 8B at a good speed.",
};

// Mock catalogue (mirrors `catalog.toml`), with fit + size already resolved for
// the mock hardware. The baked Llama-3.2-1B is installed + active on first boot.
const MOCK_MODELS: LocalModel[] = [
  {
    source: "bartowski/Llama-3.2-1B-Instruct-GGUF",
    name: "Llama 3.2 1B",
    paramsB: 1.24,
    tasks: ["general"],
    advanced: false,
    fit: "fits",
    tokensPerSec: 48,
    sizeGb: 0.8,
    installed: true,
    active: true,
    baked: true,
  },
  {
    source: "bartowski/Qwen2.5-7B-Instruct-GGUF",
    name: "Qwen2.5 7B",
    paramsB: 7.62,
    tasks: ["general"],
    advanced: false,
    fit: "fits",
    tokensPerSec: 9.4,
    sizeGb: 4.7,
    installed: false,
    active: false,
    baked: false,
  },
  {
    source: "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
    name: "Qwen2.5 Coder 7B",
    paramsB: 7.62,
    tasks: ["coding"],
    advanced: false,
    fit: "fits",
    tokensPerSec: 9.4,
    sizeGb: 4.7,
    installed: false,
    active: false,
    baked: false,
  },
  {
    source: "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
    name: "Llama 3.1 8B",
    paramsB: 8.03,
    tasks: ["general", "reasoning"],
    advanced: false,
    fit: "may-be-slow",
    tokensPerSec: 3.6,
    sizeGb: 5.0,
    installed: false,
    active: false,
    baked: false,
  },
  {
    source: "huihui-ai/Qwen3-8B-abliterated",
    name: "Qwen3 8B (unfiltered)",
    paramsB: 8.19,
    tasks: ["general"],
    advanced: true,
    fit: "may-be-slow",
    tokensPerSec: 3.4,
    sizeGb: 5.1,
    installed: false,
    active: false,
    baked: false,
  },
];

const FIT_RANK: Record<Fit, number> = { fits: 0, "may-be-slow": 1, "wont-fit": 2 };

/// The three tier picks (backend `tier_picks`): the best-fitting model for each
/// of Fast / Balanced / Quality, non-advanced only.
export function tierPicks(list: LocalModel[]): Record<Tier, LocalModel | null> {
  const usable = list
    .filter((m) => !m.advanced && m.fit !== "wont-fit")
    .sort((a, b) => a.paramsB - b.paramsB);
  const pick = (lo: number, hi: number): LocalModel | null => {
    const inBand = usable.filter((m) => m.paramsB >= lo && m.paramsB < hi);
    const band = inBand.length > 0 ? inBand : usable;
    // Prefer a better fit, then the larger model within the band.
    return (
      [...band].sort(
        (a, b) => FIT_RANK[a.fit] - FIT_RANK[b.fit] || b.paramsB - a.paramsB,
      )[0] ?? null
    );
  };
  return {
    fast: pick(0, 4),
    balanced: pick(4, 8),
    quality: pick(8, Infinity),
  };
}

/// The advanced models, revealed behind the "Advanced" door.
export function advancedModels(list: LocalModel[]): LocalModel[] {
  return list.filter((m) => m.advanced);
}

/// The installed models (for "Your models"), the active one first.
export function installedModels(list: LocalModel[]): LocalModel[] {
  return list
    .filter((m) => m.installed)
    .sort((a, b) => Number(b.active) - Number(a.active));
}

/// Load the catalogue + hardware. Prefers the real bridge; falls back to the
/// fixture while the Settings-side commands are unwired.
export async function loadLocalModels(): Promise<void> {
  try {
    hardware.set(await invoke<Hardware>("ai_hardware_probe"));
    models.set(await invoke<LocalModel[]>("ai_local_models_catalog"));
  } catch {
    hardware.set(MOCK_HARDWARE);
    models.set(MOCK_MODELS);
  } finally {
    modelsLoaded.set(true);
  }
}

let downloadTimer: ReturnType<typeof setInterval> | null = null;

/// Start a download: the one consented egress. Mirrors backend progress; the
/// mock simulates it so the flow is reviewable. Marks the model installed on
/// completion.
export async function startDownload(m: LocalModel): Promise<void> {
  try {
    await invoke("ai_local_models_download", { source: m.source });
  } catch {
    // Bridge unwired: simulate the streamed progress locally.
  }
  const total = Math.round(m.sizeGb * 1_000_000_000);
  download.set({ source: m.source, bytesFetched: 0, totalBytes: total, status: "downloading" });
  if (downloadTimer) clearInterval(downloadTimer);
  downloadTimer = setInterval(() => {
    const d = get(download);
    if (!d || d.source !== m.source) return;
    const next = d.bytesFetched + total / 40;
    if (next >= total) {
      download.set({ ...d, bytesFetched: total, status: "verifying" });
      if (downloadTimer) clearInterval(downloadTimer);
      downloadTimer = setTimeout(
        () => finishDownload(m.source),
        900,
      ) as unknown as ReturnType<typeof setInterval>;
      return;
    }
    download.set({ ...d, bytesFetched: next });
  }, 120);
}

function finishDownload(source: string) {
  models.update((list) =>
    list.map((m) => (m.source === source ? { ...m, installed: true } : m)),
  );
  download.set(null);
}

/// Cancel an in-flight download.
export async function cancelDownload(source: string): Promise<void> {
  if (downloadTimer) {
    clearInterval(downloadTimer);
    clearTimeout(downloadTimer);
  }
  download.set(null);
  try {
    await invoke("ai_local_models_cancel_download", { source });
  } catch {
    // Nothing to surface.
  }
}

/// Make an installed model the active one (one live model at a time).
export async function setActive(source: string): Promise<void> {
  models.update((list) => list.map((m) => ({ ...m, active: m.source === source })));
  try {
    await invoke("ai_local_models_set_active", { source });
  } catch {
    // Local view already reflects the switch.
  }
}

/// Delete a local model to reclaim its space. The baked default cannot be
/// deleted (it is the offline guarantee); the active model hands off first.
export async function deleteModel(source: string): Promise<void> {
  models.update((list) => {
    const wasActive = list.find((m) => m.source === source)?.active ?? false;
    let next = list.map((m) =>
      m.source === source ? { ...m, installed: false, active: false } : m,
    );
    if (wasActive) {
      const baked = next.find((m) => m.installed && m.baked);
      if (baked) next = next.map((m) => ({ ...m, active: m.source === baked.source }));
    }
    return next;
  });
  try {
    await invoke("ai_local_models_delete", { source });
  } catch {
    // Local view already reflects the removal.
  }
}
