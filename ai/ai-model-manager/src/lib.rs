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
}
