/// The Models hub: one place to pick which model answers each task, get new
/// models curated for this machine, browse/search for a specific one, and bring
/// your own from disk. Merges the old Default-models picker and the Model
/// Manager (local-model-bundle-plan.md + ai-providers-plan.md §Settings).
///
/// Non-technical framing: a plain fit verdict computed locally (never a download
/// to find out), quant hidden (the backend picks the best-fitting Q4 ladder),
/// size in GB, downloading is the one consented egress. The baked Llama-3.2-1B is
/// the offline default (real: `arlen-llama.service`).
///
/// Almost everything here is a fixture today: the daemon stores ONE active model
/// (per-role defaults are new backend), cannot yet enumerate downloaded models,
/// has no Hugging Face search and no import. The store reads the intended bridge
/// commands then falls back to the mock so the whole surface is reviewable.

import { invoke } from "@tauri-apps/api/core";
import { writable, get, derived } from "svelte/store";

/// A recommendation tier: three clear choices, hardware-picked.
export type Tier = "fast" | "balanced" | "quality";

/// The plain 3-way fit verdict from `ai-model-manager::fit_badge`.
export type Fit = "fits" | "may-be-slow" | "wont-fit";

/// The task a role fills. The daemon resolves a model per role (new contract).
export type Role = "query" | "agent" | "title";

/// A model that can run: a local GGUF (downloaded / built-in / imported) or a
/// cloud model from a connected provider.
export interface Model {
  /// Stable id, e.g. "local/llama-3.2-1b" or "anthropic/claude-3.5-sonnet".
  id: string;
  name: string;
  /// "local" for on-device, else the cloud provider id ("anthropic", "mistral").
  provider: string;
  kind: "local" | "cloud";
  tasks: string[];
  /// Local-only sizing/fit fields.
  paramsB?: number;
  fit?: Fit;
  tokensPerSec?: number;
  sizeGb?: number;
  /// Local: downloaded / built-in / imported. Cloud: the provider is connected.
  installed: boolean;
  /// The baked offline default (cannot be removed).
  baked: boolean;
  /// Brought in from the user's disk.
  imported: boolean;
  /// Uncurated community model (hidden behind the browse "Advanced" filter).
  advanced: boolean;
  /// A curated result surfaced only by an explicit Hugging Face search.
  fromSearch?: boolean;
}

/// The machine's capability, phrased for a non-technical reader.
export interface Hardware {
  ramGb: number;
  accelerator: "apu" | "discrete";
  vramGb: number | null;
  summary: string;
}

/// An in-flight download (the backend streams bytes; this mirrors progress).
export interface Download {
  id: string;
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

const ROLE_META: Record<Role, { label: string; description: string }> = {
  query: { label: "Chat", description: "Answers your questions." },
  agent: { label: "Background work", description: "Runs the tasks you have turned on." },
  title: { label: "Chat titles", description: "Names new chats. A small local model is plenty." },
};

/// The label and description for a role row.
export function roleMeta(role: Role): { label: string; description: string } {
  return ROLE_META[role];
}

/// The whole catalogue (local curated + connected cloud), the hardware line, the
/// per-role assignments, any in-flight download, and whether a Hugging Face
/// search has been run this session.
export const models = writable<Model[]>([]);
export const hardware = writable<Hardware | null>(null);
export const download = writable<Download | null>(null);
export const modelsLoaded = writable(false);
export const roles = writable<Record<Role, string>>({
  query: "local/llama-3.2-1b",
  agent: "local/llama-3.2-1b",
  title: "local/llama-3.2-1b",
});
/// Null until the user opts into a Hugging Face search; then reachable/false
/// records whether the reach succeeded (the curated list is the fallback).
export const hfSearch = writable<{ reachable: boolean } | null>(null);

// Mock hardware: a 7840U-class APU (Tim's datapoint) - 61 GB RAM, integrated
// GPU. Leads with the speed/size story, not VRAM.
const MOCK_HARDWARE: Hardware = {
  ramGb: 61,
  accelerator: "apu",
  vramGb: null,
  summary: "Your machine can run models up to about 8B at a good speed.",
};

// Mock catalogue. Local models mirror the curated `catalog.toml` shape (a larger
// set here so "Browse more" is real), with fit/size resolved for the mock
// hardware; two connected cloud models make the role pickers span local + cloud.
const MOCK_MODELS: Model[] = [
  local("llama-3.2-1b", "Llama 3.2 1B", 1.24, ["general"], "fits", 48, 0.8, { installed: true, baked: true }),
  local("llama-3.2-3b", "Llama 3.2 3B", 3.2, ["general"], "fits", 24, 2.0),
  local("phi-4-mini", "Phi-4 Mini 3B", 3.8, ["general", "reasoning"], "fits", 22, 2.3),
  local("mistral-7b", "Mistral 7B", 7.25, ["general"], "fits", 10, 4.1),
  local("qwen2.5-7b", "Qwen2.5 7B", 7.62, ["general"], "fits", 9.4, 4.7, { installed: true }),
  local("qwen2.5-coder-7b", "Qwen2.5 Coder 7B", 7.62, ["coding"], "fits", 9.4, 4.7),
  local("llama-3.1-8b", "Llama 3.1 8B", 8.03, ["general", "reasoning"], "may-be-slow", 3.6, 5.0),
  local("gemma-2-9b", "Gemma 2 9B", 9.24, ["general"], "may-be-slow", 3.2, 5.4),
  local("qwen2.5-14b", "Qwen2.5 14B", 14.7, ["general", "reasoning"], "may-be-slow", 1.8, 9.0),
  local("llama-3.3-70b", "Llama 3.3 70B", 70.6, ["general", "reasoning"], "wont-fit", 0.4, 40.0),
  local("qwen3-8b-abliterated", "Qwen3 8B (unfiltered)", 8.19, ["general"], "may-be-slow", 3.4, 5.1, { advanced: true }),
  cloud("anthropic/claude-3.5-sonnet", "Claude 3.5 Sonnet", "anthropic", ["general", "reasoning"]),
  cloud("mistral/mistral-large", "Mistral Large", "mistral", ["general"]),
];

// A local curated model. `installed` defaults false (not yet downloaded).
function local(
  slug: string,
  name: string,
  paramsB: number,
  tasks: string[],
  fit: Fit,
  tokensPerSec: number,
  sizeGb: number,
  opts: { installed?: boolean; baked?: boolean; advanced?: boolean } = {},
): Model {
  return {
    id: `local/${slug}`,
    name,
    provider: "local",
    kind: "local",
    tasks,
    paramsB,
    fit,
    tokensPerSec,
    sizeGb,
    installed: opts.installed ?? opts.baked ?? false,
    baked: opts.baked ?? false,
    imported: false,
    advanced: opts.advanced ?? false,
  };
}

// A cloud model from a connected provider (always "installed" = available).
function cloud(id: string, name: string, provider: string, tasks: string[]): Model {
  return { id, name, provider, kind: "cloud", tasks, installed: true, baked: false, imported: false, advanced: false };
}

const FIT_RANK: Record<Fit, number> = { fits: 0, "may-be-slow": 1, "wont-fit": 2 };

/// The three tier picks: the best-fitting local model for each of
/// Fast / Balanced / Quality, non-advanced only.
export function tierPicks(list: Model[]): Record<Tier, Model | null> {
  const usable = list
    .filter((m) => m.kind === "local" && !m.advanced && m.fit !== "wont-fit")
    .sort((a, b) => (a.paramsB ?? 0) - (b.paramsB ?? 0));
  const pick = (lo: number, hi: number): Model | null => {
    const inBand = usable.filter((m) => (m.paramsB ?? 0) >= lo && (m.paramsB ?? 0) < hi);
    const band = inBand.length > 0 ? inBand : usable;
    return (
      [...band].sort(
        (a, b) => FIT_RANK[a.fit ?? "fits"] - FIT_RANK[b.fit ?? "fits"] || (b.paramsB ?? 0) - (a.paramsB ?? 0),
      )[0] ?? null
    );
  };
  return { fast: pick(0, 4), balanced: pick(4, 8), quality: pick(8, Infinity) };
}

/// The installed models for "Your models" (local only: downloaded, built-in,
/// imported), the baked default first.
export const installedModels = derived(models, ($m) =>
  $m
    .filter((m) => m.kind === "local" && m.installed)
    .sort((a, b) => Number(b.baked) - Number(a.baked)),
);

/// Every model a role can be assigned to: installed local + connected cloud.
export const availableModels = derived(models, ($m) => $m.filter((m) => m.installed));

/// Look up a model by id (for labels/logos in the pickers).
export function modelById(list: Model[], id: string): Model | undefined {
  return list.find((m) => m.id === id);
}

/// Load the catalogue + hardware. Prefers the real bridge; falls back to the
/// fixture while the Settings-side commands are unwired.
export async function loadModels(): Promise<void> {
  try {
    hardware.set(await invoke<Hardware>("ai_hardware_probe"));
    models.set(await invoke<Model[]>("ai_models_catalog"));
    roles.set(await invoke<Record<Role, string>>("ai_defaults_get_roles"));
  } catch {
    hardware.set(MOCK_HARDWARE);
    models.set(MOCK_MODELS);
  } finally {
    modelsLoaded.set(true);
  }
}

/// Assign a model to a role (persisted per-role default; new backend contract).
export async function setRole(role: Role, id: string): Promise<void> {
  roles.update((r) => ({ ...r, [role]: id }));
  try {
    await invoke("ai_defaults_set_role", { role, id });
  } catch {
    // Local view already reflects the assignment.
  }
}

let downloadTimer: ReturnType<typeof setInterval> | null = null;

/// Start a download: the one consented egress. Mirrors backend progress; the
/// mock simulates it. Marks the model installed on completion.
export async function startDownload(m: Model): Promise<void> {
  try {
    await invoke("ai_local_models_download", { id: m.id });
  } catch {
    // Bridge unwired: simulate the streamed progress locally.
  }
  const total = Math.round((m.sizeGb ?? 1) * 1_000_000_000);
  download.set({ id: m.id, bytesFetched: 0, totalBytes: total, status: "downloading" });
  if (downloadTimer) clearInterval(downloadTimer);
  downloadTimer = setInterval(() => {
    const d = get(download);
    if (!d || d.id !== m.id) return;
    const next = d.bytesFetched + total / 40;
    if (next >= total) {
      download.set({ ...d, bytesFetched: total, status: "verifying" });
      if (downloadTimer) clearInterval(downloadTimer);
      downloadTimer = setTimeout(
        () => finishDownload(m.id),
        900,
      ) as unknown as ReturnType<typeof setInterval>;
      return;
    }
    download.set({ ...d, bytesFetched: next });
  }, 120);
}

function finishDownload(id: string) {
  models.update((list) => list.map((m) => (m.id === id ? { ...m, installed: true } : m)));
  download.set(null);
}

/// Cancel an in-flight download.
export async function cancelDownload(id: string): Promise<void> {
  if (downloadTimer) {
    clearInterval(downloadTimer);
    clearTimeout(downloadTimer);
  }
  download.set(null);
  try {
    await invoke("ai_local_models_download_cancel", { id });
  } catch {
    // Nothing to surface.
  }
}

/// Delete a local model to reclaim its space. The baked default is undeletable.
/// If a role pointed at it, that role falls back to the baked model.
export async function deleteModel(id: string): Promise<void> {
  models.update((list) => list.map((m) => (m.id === id ? { ...m, installed: false } : m)));
  roles.update((r) => {
    const baked = get(models).find((m) => m.baked)?.id ?? "local/llama-3.2-1b";
    const next = { ...r };
    for (const role of Object.keys(next) as Role[]) if (next[role] === id) next[role] = baked;
    return next;
  });
  try {
    await invoke("ai_local_models_delete", { id });
  } catch {
    // Local view already reflects the removal.
  }
}

/// Import a model file from disk. The mock adds a placeholder installed model;
/// the real path validates a GGUF, reads metadata, and registers it selectable.
export async function importModel(): Promise<void> {
  try {
    const added = await invoke<Model>("ai_local_models_import");
    models.update((list) => [...list, added]);
    return;
  } catch {
    // Mock: add a representative imported model.
  }
  const imported: Model = {
    id: `local/imported-${get(models).filter((m) => m.imported).length + 1}`,
    name: "My model",
    provider: "local",
    kind: "local",
    tasks: ["general"],
    paramsB: 7,
    fit: "fits",
    tokensPerSec: 9,
    sizeGb: 4.4,
    installed: true,
    baked: false,
    imported: true,
    advanced: false,
  };
  models.update((list) => [...list, imported]);
}

/// Opt in to a Hugging Face search: the deliberate reach that broadens browse
/// beyond the curated list. The mock appends a couple de-jargonized results and
/// records that the reach succeeded; the curated list is the offline fallback.
export async function searchHuggingFace(): Promise<void> {
  try {
    const found = await invoke<Model[]>("ai_models_search_hf");
    models.update((list) => [...list, ...found]);
    hfSearch.set({ reachable: true });
    return;
  } catch {
    // Mock the reach with two extra results.
  }
  const extra: Model[] = [
    { ...local("deepseek-r1-distill-7b", "DeepSeek R1 Distill 7B", 7.6, ["reasoning"], "fits", 9, 4.7), fromSearch: true },
    { ...local("smollm2-1.7b", "SmolLM2 1.7B", 1.7, ["general"], "fits", 40, 1.1), fromSearch: true },
  ];
  models.update((list) => [...list.filter((m) => !m.fromSearch), ...extra]);
  hfSearch.set({ reachable: true });
}
