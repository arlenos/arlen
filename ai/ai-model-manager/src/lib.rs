//! Hardware-aware local-model fit and speed recommender (`local-model-bundle-plan.md`).
//!
//! The pure core of the in-Settings model manager: given a machine's memory and
//! memory bandwidth and a model's parameter count, it computes whether a model
//! FITS in the available memory budget and how FAST it will run, and turns that
//! into the plain three-way badge the UI shows (`Fits` / `MaySlow` / `WontFit`).
//!
//! Two design facts from the plan drive the math, and both are reasons the
//! existing tools get APUs wrong:
//!
//! 1. **The budget is the unified-memory budget, not the BIOS VRAM number.** AMD
//!    APUs share system RAM; the Linux `amdgpu` GTT cap defaults to ~3/4 of system
//!    RAM, so a 61 GB laptop hands the iGPU ~45 GB and can hold models a discrete
//!    8 GB card never could. A discrete GPU's budget is its own VRAM.
//! 2. **Fit and speed are separate axes.** An APU is bandwidth-bound (LPDDR5X is
//!    ~80-120 GB/s), so a 30B model that "fits" in 45 GB still crawls. Speed is
//!    estimated from memory bandwidth, not compute, because each generated token
//!    streams the whole working set from memory once.
//!
//! This module is pure (no I/O): hardware detection (reading `/proc/meminfo`, the
//! GPU, the bandwidth) and the catalog/download layer are separate concerns that
//! feed [`Hardware`] and [`ModelSpec`] in. Quant jargon never leaves this layer;
//! the UI sees only the badge and the size.

/// One byte-count gibibyte (2^30), the binary unit GPU and RAM tools report. The
/// plan's anchor figures (Llama-3.1-8B Q4_K_M = 4.58 GiB, ...) are in GiB, so the
/// footprint math stays in GiB end to end.
const GIB: f64 = 1_073_741_824.0;

/// The fraction of real memory bandwidth a memory-bound llama.cpp run sustains on
/// an iGPU, well below the theoretical peak (DRAM never delivers peak under a
/// random-ish read stream, and the KV-cache adds traffic). Calibrated to Tim's
/// live 7840U datapoint: qwen2.5:7b-Q4 (~4.3 GiB working set) at ~9.4 tok/s on
/// LPDDR5X-6400 (~102 GB/s peak) implies ~0.4. Coarse by design: the output feeds
/// a three-way speed badge, not a benchmark.
const MEMORY_BANDWIDTH_EFFICIENCY: f64 = 0.4;

/// Below this generation rate a model that fits is still labelled "may be slow"
/// rather than "fits": the APU case the plan calls out, where a big model loads
/// but generation crawls. Tim's 9.4 tok/s reads as usable (above this); a 30B
/// streamed from RAM at ~2 tok/s reads as slow (below it).
const USABLE_TOKENS_PER_SEC: f64 = 4.0;

/// The fraction of system RAM the `amdgpu` GTT cap hands a unified-memory APU by
/// default (the soft, large budget that lets an APU run models a discrete card
/// cannot). The plan's load-bearing APU correction.
const APU_RAM_BUDGET_FRACTION: f64 = 0.75;

/// A runtime multiplier over the raw weight bytes covering the ~15-20% inference
/// overhead (activations, scratch) the plan lists, so "fits" is computed against
/// the real resident set rather than the optimistic weights-only figure.
const RUNTIME_OVERHEAD: f64 = 1.18;

/// Fixed GiB reserved for the OS, driver and a baseline KV-cache, added on top of
/// the overhead-scaled weights. The plan budgets ~0.5-1 GB for the OS/driver; KV
/// grows with context (and GQA models are light), so a finer context-scaled KV
/// term is a later refinement, conservative-by-omission here (it only makes "fits"
/// slightly optimistic at very long contexts).
const FIXED_RESIDENT_GIB: f64 = 1.0;

/// A GGUF quantization level. The manager applies these silently (the plan: quant
/// jargon is never shown to the user); the default is [`Quant::Q4KM`], and a
/// bigger model at a lower quant beats a smaller model at a higher one only down
/// to about Q3, so nothing below `Q3KM` is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quant {
    /// ~Q3, the floor: offered only when Q4_K_M will not fit and a larger model
    /// is wanted; below this quality degrades too far.
    Q3KM,
    /// The default. Best size/quality trade-off for local use.
    Q4KM,
    /// A higher-fidelity step when the budget allows.
    Q5KM,
    /// Higher still.
    Q6K,
    /// Near-lossless 8-bit.
    Q8_0,
}

impl Quant {
    /// Bits per weight, the measured effective rate (k-quants spend extra bits on
    /// attention and FFN tensors, so Q4_K_M is ~4.9 bpw, not the naive 4.0). The
    /// values reproduce the plan's Llama-3.1-8B GiB anchors to within rounding.
    pub fn bits_per_weight(self) -> f64 {
        match self {
            Quant::Q3KM => 3.91,
            Quant::Q4KM => 4.90,
            Quant::Q5KM => 5.70,
            Quant::Q6K => 6.57,
            Quant::Q8_0 => 8.50,
        }
    }
}

/// The accelerator a model runs on, which decides how the memory budget is read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Accelerator {
    /// A discrete GPU: the budget is its own dedicated VRAM.
    Discrete {
        /// Dedicated video memory, in GiB.
        vram_gib: f64,
    },
    /// A unified-memory APU (shares system RAM): the budget is a large fraction of
    /// total RAM, NOT the small BIOS-reserved VRAM carve-out.
    Apu,
}

/// The machine's memory profile, the recommender's input. Populated by the
/// (separate) hardware-detection layer; pure here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hardware {
    /// Total system RAM, in GiB.
    pub ram_gib: f64,
    /// How the model is accelerated, which selects the budget basis.
    pub accelerator: Accelerator,
    /// Sustainable-ish memory bandwidth, in GB/s, the speed-limiting resource on a
    /// bandwidth-bound APU (e.g. ~102 for LPDDR5X-6400 dual channel).
    pub mem_bandwidth_gbps: f64,
}

/// A model the manager can recommend: just the parameter count drives the math
/// (the name and task grouping are presentation, layered above).
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSpec {
    /// Display name (e.g. `"Llama-3.1-8B"`).
    pub name: String,
    /// Parameter count in billions (e.g. `8.03`).
    pub params_b: f64,
}

/// The plain three-way verdict the UI renders, the only thing that leaves this
/// layer besides the size. On an APU the plan says to lead with the speed word,
/// so [`FitBadge::MaySlow`] is its own state distinct from [`FitBadge::Fits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitBadge {
    /// Loads and generates at a usable rate.
    Fits,
    /// Loads within the budget but generation is slow (the bandwidth-bound case).
    MaySlow,
    /// Does not fit in the memory budget.
    WontFit,
}

/// The curated tier a recommendation is presented under (the de-facto convention:
/// Fast / Balanced / Quality). `Balanced` is the Q4_K_M default; on a
/// bandwidth-bound machine the speed axis carries the real signal between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Small and high-quant for the highest token rate.
    Fast,
    /// The Q4_K_M default: the everyday balance.
    Balanced,
    /// The largest that fits, accepting a lower token rate.
    Quality,
}

/// The weights-only resident size of `params_b` billion parameters at `quant`, in
/// GiB. This is `params * bpw / 8` in bytes, expressed in GiB; it reproduces the
/// plan's anchors (8.03B Q4_K_M -> 4.58 GiB).
pub fn weights_gib(params_b: f64, quant: Quant) -> f64 {
    params_b * 1e9 * quant.bits_per_weight() / 8.0 / GIB
}

/// The estimated resident memory footprint of running `params_b` at `quant`, in
/// GiB: the weights scaled by the runtime overhead plus the fixed OS/driver/KV
/// reservation. This is what a fit decision is made against, so it is the honest
/// resident set, not the weights-only figure.
pub fn footprint_gib(params_b: f64, quant: Quant) -> f64 {
    weights_gib(params_b, quant) * RUNTIME_OVERHEAD + FIXED_RESIDENT_GIB
}

/// The memory budget available to a model on this hardware, in GiB: a discrete
/// GPU's dedicated VRAM, or ~3/4 of system RAM on a unified-memory APU. Never the
/// BIOS VRAM number for an APU (the mistake the plan calls out).
pub fn memory_budget_gib(hw: &Hardware) -> f64 {
    match hw.accelerator {
        Accelerator::Discrete { vram_gib } => vram_gib,
        Accelerator::Apu => hw.ram_gib * APU_RAM_BUDGET_FRACTION,
    }
}

/// Parse total system RAM in GiB from `/proc/meminfo` contents: the `MemTotal:`
/// line, reported in kibibytes, converted to GiB. `None` when the line is absent
/// or unparseable. Pure, so it is unit-tested without the filesystem.
///
/// One of three pure cores that populate [`Hardware`]: this (RAM), the
/// [`classify_accelerator`] split (discrete VRAM vs APU unified), and the
/// [`theoretical_bandwidth_gbps`] formula. Still deferred is the I/O that FEEDS the
/// latter two - the `/sys/class/drm` reads for `is_integrated` + the discrete VRAM,
/// and the DMI (SMBIOS Type 17) reads for the DRAM data rate / width / channel
/// count - which need careful, privilege-sensitive grounding (a wrong value
/// mis-tiers), so they are kept out of these pure, anchored functions.
pub fn parse_meminfo_ram_gib(contents: &str) -> Option<f64> {
    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            // e.g. "MemTotal:       65802152 kB" - the first token is kibibytes.
            let kib: f64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kib * 1024.0 / GIB);
        }
    }
    None
}

/// Read total system RAM in GiB from `/proc/meminfo`. `None` on a read or parse
/// failure (the caller falls back conservatively rather than over-promising a
/// budget). Linux-only (the file is a Linux interface); the pure
/// [`parse_meminfo_ram_gib`] is testable everywhere.
#[cfg(target_os = "linux")]
pub fn detect_ram_gib() -> Option<f64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|c| parse_meminfo_ram_gib(&c))
}

/// Conservative RAM fallback in GiB when `/proc/meminfo` cannot be read: low, so
/// the budget under-promises (recommends smaller models) rather than
/// over-recommending against RAM that may not be there.
const FALLBACK_RAM_GIB: f64 = 8.0;

/// Conservative DRAM transfer rate (MT/s) for the bandwidth fallback - a
/// DDR4-3200 dual-channel baseline - used until the (privilege-sensitive, still
/// deferred) DMI/SMBIOS detection produces the real rate. Chosen LOW so an
/// undetected machine under-promises its token rate (more `MaySlow`) rather than
/// over-promising it; only the rate estimate depends on it, never the fit.
const FALLBACK_DRAM_MTPS: u32 = 3200;

/// Assemble the machine's [`Hardware`] from the detected signals, applying the
/// layer's conservative fallbacks for any signal the (privilege-sensitive, still
/// deferred) GPU/DMI detection has not produced. Pure over its input, so the
/// fallback policy is unit-tested without the filesystem: `detected_ram` is the
/// `/proc/meminfo` read ([`detect_ram_gib`]), falling back to [`FALLBACK_RAM_GIB`].
///
/// Until the `/sys/class/drm` `is_integrated` + VRAM detect lands, the accelerator
/// defaults to unified-memory [`Accelerator::Apu`]: it budgets against ~3/4 of RAM
/// ([`memory_budget_gib`]), which UNDER-counts a discrete card's dedicated VRAM
/// rather than over-promising a budget that is not there - the safe direction the
/// layer mandates (a value over-promising would mis-tier and OOM a load). Bandwidth
/// defaults to the conservative [`FALLBACK_DRAM_MTPS`] dual-channel baseline through
/// [`theoretical_bandwidth_gbps`] until the DMI detect lands. Every fallback errs
/// toward under-promising.
pub fn assemble_hardware(detected_ram: Option<f64>) -> Hardware {
    Hardware {
        ram_gib: detected_ram.unwrap_or(FALLBACK_RAM_GIB),
        accelerator: Accelerator::Apu,
        mem_bandwidth_gbps: theoretical_bandwidth_gbps(FALLBACK_DRAM_MTPS, 64, 2),
    }
}

/// Detect the machine's [`Hardware`] from the live system, with the conservative
/// fallbacks of [`assemble_hardware`] for any signal the deferred GPU/DMI layer
/// does not yet produce. Linux-only (it reads `/proc/meminfo` via
/// [`detect_ram_gib`]); the pure [`assemble_hardware`] is testable everywhere.
#[cfg(target_os = "linux")]
pub fn detect_hardware() -> Hardware {
    assemble_hardware(detect_ram_gib())
}

/// Classify the accelerator from the two signals the detection layer reads:
/// whether the GPU is INTEGRATED (an APU sharing system RAM) and, for a discrete
/// card, its dedicated VRAM in GiB. Pure: the `/sys` reads that produce
/// `is_integrated` and `vram_gib` are the (separate, careful) I/O layer.
///
/// The split is the plan's core differentiator. `is_integrated` - NOT the VRAM
/// size - is the deciding signal, precisely so a low-VRAM discrete card is not
/// mistaken for an APU (and over-budgeted against 3/4 RAM) and an APU's tiny BIOS
/// VRAM carve-out is not mistaken for a discrete budget. An integrated GPU is
/// unified-memory; only a discrete card's budget is its own VRAM.
pub fn classify_accelerator(is_integrated: bool, vram_gib: f64) -> Accelerator {
    if is_integrated {
        Accelerator::Apu
    } else {
        Accelerator::Discrete { vram_gib }
    }
}

/// The theoretical peak memory bandwidth in GB/s (decimal) for a memory
/// configuration: `data_rate_mtps` mega-transfers/second, `bus_width_bits` data
/// width per channel (64 for a standard DDR channel), across `channels` channels.
/// This is transfers/s times bytes-per-transfer-per-channel times channels: the
/// speed-limiting resource on a bandwidth-bound APU that the recommender estimates
/// the token rate from. The plan's anchor - LPDDR5X-6400, 64-bit channels,
/// dual-channel - yields ~102 GB/s.
///
/// This is the PEAK the DRAM config can move; the sustained fraction a real
/// llama.cpp run achieves is applied separately ([`MEMORY_BANDWIDTH_EFFICIENCY`]).
/// Returns 0 for a degenerate (zero) config so the caller falls back rather than
/// trusting a bogus peak. Pure: the DMI/`/sys` reads that produce the data rate,
/// width and channel count are the (separate, careful, privilege-sensitive) I/O
/// layer - a wrong value here mis-tiers, so the inputs are grounded, not guessed.
pub fn theoretical_bandwidth_gbps(data_rate_mtps: u32, bus_width_bits: u32, channels: u32) -> f64 {
    if data_rate_mtps == 0 || bus_width_bits == 0 || channels == 0 {
        return 0.0;
    }
    // (data_rate_mtps * 1e6 transfers/s) * (bus_width_bits/8 bytes) * channels,
    // expressed in GB/s (1e9 bytes/s): the 1e6/1e9 collapses to /1000.
    data_rate_mtps as f64 * (bus_width_bits as f64 / 8.0) * channels as f64 / 1000.0
}

/// Estimated generation rate in tokens/second for a model whose per-token streamed
/// size is `streamed_gib` (the WEIGHTS, the bytes read from memory for each token,
/// NOT the full resident footprint: the OS/driver reservation is not re-read per
/// token), on a machine with `mem_bandwidth_gbps` memory bandwidth. Memory-bound
/// model: each token streams the weights once, so rate scales as bandwidth / size,
/// derated by [`MEMORY_BANDWIDTH_EFFICIENCY`]. Coarse (it feeds a three-way badge),
/// and meaningless for a zero/negative size, which returns 0.
pub fn estimate_tokens_per_sec(streamed_gib: f64, mem_bandwidth_gbps: f64) -> f64 {
    if streamed_gib <= 0.0 {
        return 0.0;
    }
    mem_bandwidth_gbps * MEMORY_BANDWIDTH_EFFICIENCY / streamed_gib
}

/// The fit badge for `params_b` at `quant` on `hw`: `WontFit` if the footprint
/// exceeds the budget, else `MaySlow` when the estimated rate is below the usable
/// threshold (the bandwidth-bound APU case), else `Fits`.
pub fn fit_badge(params_b: f64, quant: Quant, hw: &Hardware) -> FitBadge {
    let footprint = footprint_gib(params_b, quant);
    if footprint > memory_budget_gib(hw) {
        return FitBadge::WontFit;
    }
    // Speed is bound by the per-token streamed bytes (the weights), not the full
    // resident footprint (the fixed OS/driver reservation is not re-read per token).
    let tok_s = estimate_tokens_per_sec(weights_gib(params_b, quant), hw.mem_bandwidth_gbps);
    if tok_s < USABLE_TOKENS_PER_SEC {
        FitBadge::MaySlow
    } else {
        FitBadge::Fits
    }
}

/// The quant the manager would silently pick for `params_b` on `hw`: the default
/// Q4_K_M when it fits, stepping DOWN the ladder toward the Q3 floor only to make a
/// model that would otherwise not fit fit. Returns `None` when even the Q3 floor
/// does not fit (the model is too big for this machine at any sane quant). Never
/// steps below `Q3KM` (the plan: avoid IQ2/Q2).
pub fn best_fitting_quant(params_b: f64, hw: &Hardware) -> Option<Quant> {
    let budget = memory_budget_gib(hw);
    // Default first, then degrade only as far as the Q3 floor.
    [Quant::Q4KM, Quant::Q3KM]
        .into_iter()
        .find(|&quant| footprint_gib(params_b, quant) <= budget)
}

/// A recommendation for one model on the target hardware: the quant the manager
/// would silently apply, the resulting fit badge, and the estimated generation
/// rate. `quant` is `None` only when the model does not fit at any sane quant (the
/// badge is then `WontFit` and the rate 0).
#[derive(Debug, Clone, PartialEq)]
pub struct Recommendation {
    /// The model's display name.
    pub name: String,
    /// The parameter count in billions.
    pub params_b: f64,
    /// The quant the manager would apply (the Q4_K_M-default ladder), or `None`
    /// when nothing fits.
    pub quant: Option<Quant>,
    /// The plain three-way verdict.
    pub badge: FitBadge,
    /// Estimated tokens/second at the applied quant (0 when nothing fits).
    pub tokens_per_sec: f64,
}

/// A task grouping the curated catalog organizes models under (the UI shows "good
/// for writing / coding"; `General` is the everyday default). Presentation only -
/// the fit/speed math does not depend on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Task {
    /// The everyday assistant default.
    General,
    /// Prose / writing assistance.
    Writing,
    /// Code generation and editing.
    Coding,
    /// Step-by-step reasoning / math.
    Reasoning,
}

/// A curated catalog entry: the recommender input (`name` + `params_b`) plus the
/// presentation and provenance the manager needs - the task groups it shows under,
/// the GGUF `source` it is fetched from, and whether it is an `advanced`
/// (abliterated / uncensored grey-zone) model gated behind the explicit Advanced
/// door (plan Decision 3). The curation itself - which models, the real source
/// refs, the advanced set - is human-vetted and lives in the bundled catalog TOML;
/// this is its schema + parser, not the curated list.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct CuratedModel {
    /// Display name (e.g. `"Llama-3.1-8B"`).
    pub name: String,
    /// Parameter count in billions.
    pub params_b: f64,
    /// The task groups the UI lists this model under (empty = General only).
    #[serde(default)]
    pub tasks: Vec<Task>,
    /// The GGUF source reference the downloader fetches from (e.g. a Hugging Face
    /// repo id). Opaque here; the consent-gated downloader resolves + fetches it.
    pub source: String,
    /// Whether this is an advanced (abliterated / uncensored) model, hidden behind
    /// the explicit Advanced door and excluded from the default curated shortlist.
    #[serde(default)]
    pub advanced: bool,
}

impl CuratedModel {
    /// The recommender input view (name + parameter count) for this entry.
    pub fn spec(&self) -> ModelSpec {
        ModelSpec {
            name: self.name.clone(),
            params_b: self.params_b,
        }
    }
}

/// The curated catalog: the manager's source of recommendable models. Parsed from
/// the bundled catalog TOML (a `[[model]]` array).
#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize)]
pub struct Catalog {
    /// The curated models, in catalog order.
    #[serde(default, rename = "model")]
    pub models: Vec<CuratedModel>,
}

/// A catalog parse failure (malformed TOML or a missing required field).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError(pub String);

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "catalog parse error: {}", self.0)
    }
}

impl std::error::Error for CatalogError {}

/// Parse a curated catalog from TOML (`[[model]]` entries). Fails closed on
/// malformed TOML or a missing required field (`name`/`params_b`/`source`) rather
/// than silently dropping a model. Pure, so it is unit-tested without a file.
pub fn parse_catalog(toml_contents: &str) -> Result<Catalog, CatalogError> {
    toml::from_str(toml_contents).map_err(|e| CatalogError(e.to_string()))
}

/// Recommend the catalog for `hw`. The default view hides the `advanced`
/// (abliterated) tier; `include_advanced` is set only behind the explicit Advanced
/// door. Order preserved, advanced entries filtered before the recommender runs.
pub fn recommend_catalog(
    hw: &Hardware,
    catalog: &Catalog,
    include_advanced: bool,
) -> Vec<Recommendation> {
    let specs: Vec<ModelSpec> = catalog
        .models
        .iter()
        .filter(|m| include_advanced || !m.advanced)
        .map(CuratedModel::spec)
        .collect();
    recommend(hw, &specs)
}

/// Recommend every model in `catalog` for `hw`: per model, pick the silent quant
/// (the Q4_K_M-default ladder), the fit badge, and the speed estimate. Order
/// preserved. Pure.
pub fn recommend(hw: &Hardware, catalog: &[ModelSpec]) -> Vec<Recommendation> {
    catalog
        .iter()
        .map(|m| {
            let quant = best_fitting_quant(m.params_b, hw);
            let (badge, tokens_per_sec) = match quant {
                Some(q) => (
                    fit_badge(m.params_b, q, hw),
                    estimate_tokens_per_sec(weights_gib(m.params_b, q), hw.mem_bandwidth_gbps),
                ),
                None => (FitBadge::WontFit, 0.0),
            };
            Recommendation {
                name: m.name.clone(),
                params_b: m.params_b,
                quant,
                badge,
                tokens_per_sec,
            }
        })
        .collect()
}

/// The curated tier picks the manager surfaces for a catalog on the target
/// hardware (the Fast / Balanced / Quality convention). A tier is `None` when no
/// model qualifies for it (an empty catalog, or nothing fits).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TierPicks {
    /// Smallest footprint at the highest token rate (the snappiest).
    pub fast: Option<Recommendation>,
    /// The everyday default: the largest model that still runs at a usable rate
    /// (`Fits`, not `MaySlow`). Falls back to the `fast` pick when nothing reads
    /// `Fits` (a bandwidth-bound machine where even the small models are slow).
    pub balanced: Option<Recommendation>,
    /// The largest model that fits at all, accepting a lower token rate.
    pub quality: Option<Recommendation>,
}

/// Pick the Fast / Balanced / Quality models from `catalog` for `hw`. Only models
/// that fit (badge is not `WontFit`) are considered; on a bandwidth-bound APU the
/// speed axis is what separates the tiers (most models "fit", so leading on speed
/// is the real signal the plan calls for). Pure.
pub fn tier_picks(hw: &Hardware, catalog: &[ModelSpec]) -> TierPicks {
    let fitting: Vec<Recommendation> = recommend(hw, catalog)
        .into_iter()
        .filter(|r| r.badge != FitBadge::WontFit)
        .collect();
    let fast = fitting
        .iter()
        .max_by(|a, b| a.tokens_per_sec.total_cmp(&b.tokens_per_sec))
        .cloned();
    let quality = fitting
        .iter()
        .max_by(|a, b| a.params_b.total_cmp(&b.params_b))
        .cloned();
    let balanced = fitting
        .iter()
        .filter(|r| r.badge == FitBadge::Fits)
        .max_by(|a, b| a.params_b.total_cmp(&b.params_b))
        .cloned()
        .or_else(|| fast.clone());
    TierPicks {
        fast,
        balanced,
        quality,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bpw table reproduces the plan's Llama-3.1-8B GiB anchors (4.58 / 5.33 /
    /// 6.14 / 7.95) within rounding, so the footprint math is grounded in measured
    /// quant sizes rather than the naive params x 4 / 8.
    #[test]
    fn weights_match_the_plan_anchors() {
        let p = 8.03;
        assert!((weights_gib(p, Quant::Q4KM) - 4.58).abs() < 0.05);
        assert!((weights_gib(p, Quant::Q5KM) - 5.33).abs() < 0.06);
        assert!((weights_gib(p, Quant::Q6K) - 6.14).abs() < 0.06);
        assert!((weights_gib(p, Quant::Q8_0) - 7.95).abs() < 0.06);
    }

    /// An APU's budget is ~3/4 of system RAM, not the BIOS VRAM number: a 61 GB
    /// 7840U hands the iGPU ~45 GB (the plan's figure).
    fn apu_7840u() -> Hardware {
        Hardware {
            ram_gib: 61.0,
            accelerator: Accelerator::Apu,
            mem_bandwidth_gbps: 102.4,
        }
    }

    #[test]
    fn apu_budget_is_three_quarters_of_ram() {
        let b = memory_budget_gib(&apu_7840u());
        assert!(b > 45.0 && b < 46.0, "expected ~45.75, got {b}");
    }

    #[test]
    fn discrete_budget_is_the_vram() {
        let hw = Hardware {
            ram_gib: 32.0,
            accelerator: Accelerator::Discrete { vram_gib: 8.0 },
            mem_bandwidth_gbps: 448.0,
        };
        assert_eq!(memory_budget_gib(&hw), 8.0);
    }

    /// The speed estimate reproduces Tim's live datapoint: a 7B-Q4 (~7.6B params,
    /// qwen2.5) at ~9.4 tok/s on the 7840U.
    #[test]
    fn speed_matches_the_live_7b_datapoint() {
        // Speed tracks the streamed weights, not the resident footprint.
        let streamed = weights_gib(7.6, Quant::Q4KM);
        let tok_s = estimate_tokens_per_sec(streamed, 102.4);
        assert!((tok_s - 9.4).abs() < 1.5, "expected ~9.4 tok/s, got {tok_s}");
    }

    /// On the APU, a 7B fits AND is usable, while a 30B fits the 45 GB budget but
    /// is bandwidth-bound to a slow rate (the fit-vs-speed split the plan stresses).
    #[test]
    fn fit_badge_separates_fit_from_speed_on_the_apu() {
        let hw = apu_7840u();
        assert_eq!(fit_badge(7.6, Quant::Q4KM, &hw), FitBadge::Fits);
        assert_eq!(fit_badge(30.0, Quant::Q4KM, &hw), FitBadge::MaySlow);
        // A 120B model exceeds even the 45 GB unified budget at Q4.
        assert_eq!(fit_badge(120.0, Quant::Q4KM, &hw), FitBadge::WontFit);
    }

    /// A small discrete 8 GB card runs a 7B comfortably but cannot hold a 30B at
    /// any sane quant: the APU's unified budget is the differentiator.
    #[test]
    fn small_discrete_card_cannot_hold_what_the_apu_can() {
        let hw = Hardware {
            ram_gib: 32.0,
            accelerator: Accelerator::Discrete { vram_gib: 8.0 },
            mem_bandwidth_gbps: 448.0,
        };
        assert_eq!(fit_badge(7.6, Quant::Q4KM, &hw), FitBadge::Fits);
        assert_eq!(fit_badge(30.0, Quant::Q4KM, &hw), FitBadge::WontFit);
        assert_eq!(best_fitting_quant(30.0, &hw), None);
    }

    /// The silent quant ladder: Q4_K_M by default when it fits, stepping down to
    /// the Q3 floor only to rescue a model that would otherwise not fit, never below.
    #[test]
    fn quant_ladder_defaults_to_q4_then_degrades_to_the_q3_floor() {
        let hw = apu_7840u();
        assert_eq!(best_fitting_quant(7.6, &hw), Some(Quant::Q4KM));

        // A model that fits at Q3 but not Q4 picks the floor. Pick a budget that
        // straddles the two footprints for a mid-size model.
        let params = 30.0;
        let tight = Hardware {
            ram_gib: 0.0,
            accelerator: Accelerator::Discrete {
                vram_gib: footprint_gib(params, Quant::Q3KM) + 0.1,
            },
            mem_bandwidth_gbps: 200.0,
        };
        assert!(footprint_gib(params, Quant::Q4KM) > memory_budget_gib(&tight));
        assert_eq!(best_fitting_quant(params, &tight), Some(Quant::Q3KM));
    }

    fn model(name: &str, params_b: f64) -> ModelSpec {
        ModelSpec {
            name: name.to_string(),
            params_b,
        }
    }

    #[test]
    fn tier_picks_separate_fast_balanced_quality_on_the_apu() {
        let hw = apu_7840u();
        let catalog = [
            model("tiny", 1.0),
            model("mid", 7.6),
            model("big", 30.0),
            model("huge", 120.0),
        ];
        let picks = tier_picks(&hw, &catalog);
        // Fast = the snappiest (the 1B, highest tok/s).
        assert_eq!(picks.fast.unwrap().name, "tiny");
        // Quality = the largest that fits (the 30B; the 120B is WontFit on 45 GB).
        assert_eq!(picks.quality.unwrap().name, "big");
        // Balanced = the largest that still reads Fits (the 7.6B; the 30B is the
        // bandwidth-bound MaySlow, so it is not the everyday default).
        assert_eq!(picks.balanced.unwrap().name, "mid");
    }

    #[test]
    fn recommend_marks_an_oversized_model_wontfit() {
        let hw = apu_7840u();
        let recs = recommend(&hw, &[model("huge", 120.0)]);
        assert_eq!(recs[0].badge, FitBadge::WontFit);
        assert_eq!(recs[0].quant, None);
        assert_eq!(recs[0].tokens_per_sec, 0.0);
    }

    #[test]
    fn tier_picks_are_none_when_nothing_fits() {
        let hw = apu_7840u();
        let picks = tier_picks(&hw, &[model("huge", 120.0)]);
        assert!(picks.fast.is_none() && picks.balanced.is_none() && picks.quality.is_none());
    }

    #[test]
    fn parses_memtotal_from_proc_meminfo() {
        let contents = "MemTotal:       65802152 kB\nMemFree:         1234567 kB\nSwapTotal: 0 kB\n";
        let gib = parse_meminfo_ram_gib(contents).unwrap();
        // 65802152 kiB is a 64 GB machine's MemTotal, ~62.7 GiB after reserved.
        assert!((gib - 62.75).abs() < 0.5, "expected ~62.7 GiB, got {gib}");
    }

    #[test]
    fn meminfo_without_a_parseable_memtotal_is_none() {
        assert_eq!(parse_meminfo_ram_gib("MemFree: 100 kB\n"), None);
        assert_eq!(parse_meminfo_ram_gib(""), None);
        assert_eq!(parse_meminfo_ram_gib("MemTotal: notanumber kB\n"), None);
    }

    #[test]
    fn assemble_hardware_uses_detected_ram_and_conservative_fallbacks() {
        let hw = assemble_hardware(Some(64.0));
        assert_eq!(hw.ram_gib, 64.0);
        // GPU detect deferred -> unified-memory APU (budgets against 3/4 RAM, the
        // under-promising default), not a phantom discrete VRAM budget.
        assert_eq!(hw.accelerator, Accelerator::Apu);
        assert_eq!(memory_budget_gib(&hw), 64.0 * APU_RAM_BUDGET_FRACTION);
        // Bandwidth is the conservative DDR4-3200 dual-channel baseline.
        assert_eq!(
            hw.mem_bandwidth_gbps,
            theoretical_bandwidth_gbps(FALLBACK_DRAM_MTPS, 64, 2)
        );
    }

    #[test]
    fn assemble_hardware_falls_back_to_a_low_ram_figure_when_detection_fails() {
        // No /proc/meminfo read -> the low fallback, so the budget under-promises
        // rather than over-recommending against RAM that may not exist.
        let hw = assemble_hardware(None);
        assert_eq!(hw.ram_gib, FALLBACK_RAM_GIB);
        assert_eq!(hw.accelerator, Accelerator::Apu);
    }

    #[test]
    fn an_integrated_gpu_is_an_apu_a_discrete_one_keeps_its_vram() {
        // The deciding signal is is_integrated, NOT the VRAM size: an APU's tiny
        // BIOS carve-out must not be read as a discrete budget, and a low-VRAM
        // discrete card must not be over-budgeted as unified memory.
        assert_eq!(classify_accelerator(true, 0.5), Accelerator::Apu);
        assert_eq!(classify_accelerator(false, 12.0), Accelerator::Discrete { vram_gib: 12.0 });
        // A 4 GB discrete card stays discrete (budget = its 4 GB), not an APU.
        assert_eq!(classify_accelerator(false, 4.0), Accelerator::Discrete { vram_gib: 4.0 });
    }

    #[test]
    fn bandwidth_matches_the_lpddr5x_anchor() {
        // The plan's anchor: LPDDR5X-6400, 64-bit channels, dual-channel -> ~102 GB/s.
        let bw = theoretical_bandwidth_gbps(6400, 64, 2);
        assert!((bw - 102.4).abs() < 0.1, "expected ~102 GB/s, got {bw}");
        // A single 64-bit DDR5-4800 channel is ~38.4 GB/s.
        assert!((theoretical_bandwidth_gbps(4800, 64, 1) - 38.4).abs() < 0.1);
    }

    #[test]
    fn a_degenerate_memory_config_has_zero_bandwidth() {
        assert_eq!(theoretical_bandwidth_gbps(0, 64, 2), 0.0);
        assert_eq!(theoretical_bandwidth_gbps(6400, 0, 2), 0.0);
        assert_eq!(theoretical_bandwidth_gbps(6400, 64, 0), 0.0);
    }

    #[test]
    fn the_anchor_bandwidth_reproduces_the_live_7b_token_rate() {
        // End to end: the detected bandwidth (102 GB/s) feeds the speed estimate,
        // reproducing Tim's live datapoint (7B-Q4 ~9.4 tok/s) the recommender tiers on.
        let bw = theoretical_bandwidth_gbps(6400, 64, 2);
        let tok_s = estimate_tokens_per_sec(weights_gib(8.03, Quant::Q4KM), bw);
        assert!((tok_s - 9.4).abs() < 1.5, "expected ~9.4 tok/s, got {tok_s}");
    }

    // A representative catalog fixture (schema exercise, NOT the shipped curation).
    const CATALOG_TOML: &str = r#"
        [[model]]
        name = "Llama-3.1-8B"
        params_b = 8.03
        tasks = ["general", "writing"]
        source = "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF"

        [[model]]
        name = "Qwen2.5-Coder-7B"
        params_b = 7.62
        tasks = ["coding"]
        source = "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF"

        [[model]]
        name = "Llama-3.1-8B-abliterated"
        params_b = 8.03
        source = "some/abliterated-GGUF"
        advanced = true
    "#;

    #[test]
    fn parses_a_curated_catalog() {
        let catalog = parse_catalog(CATALOG_TOML).expect("valid catalog");
        assert_eq!(catalog.models.len(), 3);
        let first = &catalog.models[0];
        assert_eq!(first.name, "Llama-3.1-8B");
        assert!((first.params_b - 8.03).abs() < 1e-9);
        assert_eq!(first.tasks, vec![Task::General, Task::Writing]);
        assert!(!first.advanced);
        assert_eq!(first.spec(), ModelSpec { name: "Llama-3.1-8B".into(), params_b: 8.03 });
        // The abliterated entry is flagged advanced; a task-less entry defaults empty.
        assert!(catalog.models[2].advanced);
        assert!(catalog.models[1].tasks == vec![Task::Coding]);
    }

    #[test]
    fn the_advanced_tier_is_hidden_unless_the_door_is_open() {
        let catalog = parse_catalog(CATALOG_TOML).unwrap();
        let hw = apu_7840u();
        // Default view: the abliterated model is excluded.
        let shortlist = recommend_catalog(&hw, &catalog, false);
        assert_eq!(shortlist.len(), 2);
        assert!(shortlist.iter().all(|r| !r.name.contains("abliterated")));
        // Behind the Advanced door: all three appear.
        let full = recommend_catalog(&hw, &catalog, true);
        assert_eq!(full.len(), 3);
    }

    #[test]
    fn a_malformed_catalog_fails_closed() {
        // Missing the required `source` field is rejected, not silently dropped.
        assert!(parse_catalog("[[model]]\nname = \"x\"\nparams_b = 7.0\n").is_err());
        // Not even TOML.
        assert!(parse_catalog("}{ not toml").is_err());
    }
}
