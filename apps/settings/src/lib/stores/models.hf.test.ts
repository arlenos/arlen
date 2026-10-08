/// The Hugging Face search reads the hub's real hit shape (`HfHit`: id,
/// downloads, likes) and turns each into an uncurated, not-installed picker
/// card. A refusal from a real host is the hub being out of reach, never two
/// sample results dressed as an answer (design-system 6.11, thread three).
import { describe, expect, it, vi, beforeEach } from "vitest";
import { get } from "svelte/store";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a) }));
vi.mock("$lib/tauri", () => ({ tauriAvailable: true }));

const { searchHuggingFace, models, hfSearch } = await import("./models");

describe("searchHuggingFace with a host", () => {
  beforeEach(() => {
    invoke.mockReset();
    models.set([]);
    hfSearch.set(null);
  });

  it("maps each hit to an uncurated search card", async () => {
    invoke.mockResolvedValueOnce([{ id: "bartowski/Qwen2.5-7B-Instruct-GGUF", downloads: 10, likes: 2 }]);
    await searchHuggingFace("qwen");
    const [m] = get(models);
    expect(m.id).toBe("hf/bartowski/Qwen2.5-7B-Instruct-GGUF");
    expect(m.name).toBe("Qwen2.5-7B-Instruct-GGUF");
    expect(m).toMatchObject({ kind: "local", installed: false, advanced: true, fromSearch: true });
    expect(get(hfSearch)).toEqual({ reachable: true });
  });

  it("reports a refusal as out of reach and adds nothing", async () => {
    invoke.mockRejectedValueOnce("network unreachable");
    await searchHuggingFace("qwen");
    expect(get(models)).toEqual([]);
    expect(get(hfSearch)).toEqual({ reachable: false });
  });
});
