//! The per-provider sovereignty info line: three factual chips plus honesty
//! flags that let the user SEE a provider's data-governance tradeoff and choose
//! (ai-providers-plan.md, "the sovereign gold"). The discipline is offer, never
//! shame: this renders facts, it never grays-out or lectures.
//!
//! This info is HAND-CURATED and goes stale fast (an acquisition, a terms flip,
//! a tier change): it is NOT in the models.dev seed (which carries only the
//! endpoints/models/pricing plumbing). Anything not yet verified is `Unknown`,
//! which renders honestly as a stub rather than a fabricated guarantee.

use serde::{Deserialize, Serialize};

/// The legal jurisdiction a provider's data processing falls under; the chip the
/// user reads first. `Unknown` until curated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Jurisdiction {
    /// Runs on the user's own device (a local runtime like Ollama / llama.cpp):
    /// the data never leaves the machine, the most sovereign tier.
    Local,
    /// United States (CLOUD Act reach).
    Us,
    /// European Union (GDPR; residency detail in [`Residency`]).
    Eu,
    /// China (National Intelligence Law).
    Cn,
    /// A jurisdiction outside the three headline blocs.
    Other,
    /// Not yet curated.
    #[default]
    Unknown,
}

impl Jurisdiction {
    /// The chip token (`US` / `EU` / `CN` / ...).
    fn chip(self) -> &'static str {
        match self {
            Jurisdiction::Local => "local",
            Jurisdiction::Us => "US",
            Jurisdiction::Eu => "EU",
            Jurisdiction::Cn => "CN",
            Jurisdiction::Other => "other",
            Jurisdiction::Unknown => "unknown",
        }
    }
}

/// The EU data-residency nuance: a hard EU-confined provider versus one whose EU
/// residency is only a policy default its DPA permits egress from (rendered with
/// a trailing `*`). Only meaningful for [`Jurisdiction::Eu`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Residency {
    /// Hard EU-confined (IONOS / Scaleway / STACKIT).
    EuConfined,
    /// EU by policy default; the DPA permits egress (Mistral / Nebius default) -
    /// rendered `EU*`.
    EuPolicyDefault,
    /// No residency claim to surface.
    #[default]
    Unspecified,
}

/// Whether the provider trains on your data. The load-bearing caveat is the
/// [`TrainsOnYou::PaidApiOnly`] `no*`: the mainstream closed providers do not
/// train on the paid API / enterprise tier but DO train on the free/consumer
/// tier by default, so a plain "no" would overstate a soft guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrainsOnYou {
    /// Never trains on your data.
    No,
    /// No on the paid API only; the free/consumer tier trains (rendered `no*`).
    PaidApiOnly,
    /// Trains on your data.
    Yes,
    /// Not yet curated (a terms-pull is owed before printing this as fact).
    #[default]
    Unknown,
}

impl TrainsOnYou {
    /// The chip value (`no` / `no*` / `yes` / `unknown`).
    fn chip(self) -> &'static str {
        match self {
            TrainsOnYou::No => "no",
            TrainsOnYou::PaidApiOnly => "no*",
            TrainsOnYou::Yes => "yes",
            TrainsOnYou::Unknown => "unknown",
        }
    }
}

/// The hand-curated sovereignty facts for one provider. All fields default to the
/// most conservative honest value (`Unknown` / `false`), so an entry that omits
/// them renders as an un-curated stub, never a fabricated assurance.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SovereigntyInfo {
    /// The legal jurisdiction chip.
    #[serde(default)]
    pub jurisdiction: Jurisdiction,
    /// The EU residency nuance (only meaningful when `jurisdiction = eu`).
    #[serde(default)]
    pub residency: Residency,
    /// Whether the provider trains on your data.
    #[serde(default)]
    pub trains_on_you: TrainsOnYou,
    /// Whether the served models are open-weight - makes the escape hatch VISIBLE
    /// (a US host of an open-weight model is an interchangeable vendor, not lock-in).
    #[serde(default)]
    pub open_weight: bool,
    /// This is an aggregator: the real jurisdiction is inherited from the
    /// downstream provider the request is relayed to (OpenRouter).
    #[serde(default)]
    pub aggregator_hop: bool,
    /// The provider is self-host-required, not a hosted API (Aleph Alpha PhariaAI).
    #[serde(default)]
    pub self_host_required: bool,
    /// The no-train claim is gated behind a zero-data-retention toggle the user
    /// must explicitly set; it is not the default.
    #[serde(default)]
    pub zdr_gated: bool,
}

impl SovereigntyInfo {
    /// The jurisdiction chip value for the picker (`"eu"`/`"us"`/`"cn"`), or `None`
    /// for local/other/unknown (the picker shows no jurisdiction chip). These exact
    /// strings match the Settings picker's `Provider.jurisdiction` contract, so the
    /// mapping is explicit here rather than a serde derive.
    pub fn jurisdiction_value(&self) -> Option<&'static str> {
        match self.jurisdiction {
            Jurisdiction::Eu => Some("eu"),
            Jurisdiction::Us => Some("us"),
            Jurisdiction::Cn => Some("cn"),
            Jurisdiction::Local | Jurisdiction::Other | Jurisdiction::Unknown => None,
        }
    }

    /// The trains-on-you chip value (`"no"`/`"no-paid"`/`"yes"`), or `None` when
    /// uncurated. Matches the picker's `Provider.trainsOnYou` contract - note
    /// `PaidApiOnly` renders `"no-paid"`, not the enum's serde name.
    pub fn trains_value(&self) -> Option<&'static str> {
        match self.trains_on_you {
            TrainsOnYou::No => Some("no"),
            TrainsOnYou::PaidApiOnly => Some("no-paid"),
            TrainsOnYou::Yes => Some("yes"),
            TrainsOnYou::Unknown => None,
        }
    }

    /// Render the human-facing info line, e.g.
    /// `[jurisdiction: EU*] · [trains on you: no*] · [open-weight: yes] · [self-host]`.
    /// Facts only; the caller decides. The three primary chips are always present;
    /// the honesty flags append only when set.
    pub fn info_line(&self) -> String {
        let jurisdiction = match (self.jurisdiction, self.residency) {
            // The EU* residency star rides on the jurisdiction chip.
            (Jurisdiction::Eu, Residency::EuPolicyDefault) => "EU*".to_string(),
            (Jurisdiction::Eu, Residency::EuConfined) => "EU".to_string(),
            (j, _) => j.chip().to_string(),
        };
        let open_weight = if self.open_weight { "yes" } else { "no" };
        let mut line = format!(
            "[jurisdiction: {jurisdiction}] · [trains on you: {}] · [open-weight: {open_weight}]",
            self.trains_on_you.chip(),
        );
        if self.aggregator_hop {
            line.push_str(" · [aggregator-hop]");
        }
        if self.self_host_required {
            line.push_str(" · [self-host]");
        }
        if self.zdr_gated {
            line.push_str(" · [ZDR-gated]");
        }
        line
    }
}

/// The hand-curated sovereignty facts for a known provider id, or `None` for one
/// not yet curated (which keeps the conservative `Unknown` stub). This is the
/// layer models.dev cannot supply: jurisdiction / trains-on-you / residency are
/// governance facts, not plumbing, and they go stale fast (an acquisition, a
/// tier-terms flip), so this table is re-verified on a cadence and stays
/// deliberately small - only facts stated plainly, murky ones left `Unknown`.
///
/// The load-bearing honesty: the mainstream closed providers (OpenAI / Anthropic
/// / Google / Mistral) do NOT train on the paid API but DO train the free tier by
/// default, encoded as [`TrainsOnYou::PaidApiOnly`] (`no*`). Providers whose
/// training/retention terms need a pull before printing as fact (DeepSeek, Qwen,
/// Together, xAI, Perplexity, Cohere) keep `trains_on_you = Unknown`.
pub fn curated_for(provider_id: &str) -> Option<SovereigntyInfo> {
    // A closed US major: US jurisdiction, no-train on the paid API only.
    let us_closed_paid = |open_weight| SovereigntyInfo {
        jurisdiction: Jurisdiction::Us,
        trains_on_you: TrainsOnYou::PaidApiOnly,
        open_weight,
        ..Default::default()
    };
    let info = match provider_id {
        "openai" | "azure" => us_closed_paid(false),
        "anthropic" => us_closed_paid(false),
        "google" | "google-vertex" | "google-vertex-anthropic" => us_closed_paid(false),
        // Mistral (FR): EU by policy default (DPA permits egress), no-train paid
        // only, open-weight models.
        "mistral" => SovereigntyInfo {
            jurisdiction: Jurisdiction::Eu,
            residency: Residency::EuPolicyDefault,
            trains_on_you: TrainsOnYou::PaidApiOnly,
            open_weight: true,
            ..Default::default()
        },
        // IONOS (DE): hard EU-confined, open-weight (Teuken/OpenGPT-X), no training.
        "ionos" => SovereigntyInfo {
            jurisdiction: Jurisdiction::Eu,
            residency: Residency::EuConfined,
            trains_on_you: TrainsOnYou::No,
            open_weight: true,
            ..Default::default()
        },
        // Aleph Alpha (DE): Cohere(CA)-acquired, now self-host-only (PhariaAI) - no
        // longer a clean hosted "German sovereign", so the self-host flag is the fact.
        "alephalpha" | "aleph-alpha" => SovereigntyInfo {
            jurisdiction: Jurisdiction::Eu,
            self_host_required: true,
            open_weight: true,
            ..Default::default()
        },
        // OpenRouter: an aggregator - the real jurisdiction is inherited from the
        // downstream provider each request is relayed to.
        "openrouter" => SovereigntyInfo {
            jurisdiction: Jurisdiction::Unknown,
            aggregator_hop: true,
            ..Default::default()
        },
        // Chinese open-weight labs: CN jurisdiction, open-weight models, but the
        // paid-API training terms are murky until pulled -> trains_on_you Unknown.
        "deepseek" => SovereigntyInfo {
            jurisdiction: Jurisdiction::Cn,
            open_weight: true,
            ..Default::default()
        },
        "alibaba" | "alibaba-cn" => SovereigntyInfo {
            jurisdiction: Jurisdiction::Cn,
            open_weight: true,
            ..Default::default()
        },
        _ => return None,
    };
    Some(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_a_conservative_uncurated_stub() {
        let info = SovereigntyInfo::default();
        assert_eq!(info.jurisdiction, Jurisdiction::Unknown);
        assert_eq!(info.trains_on_you, TrainsOnYou::Unknown);
        assert!(!info.open_weight);
        // An un-curated entry never fabricates a guarantee.
        assert_eq!(
            info.info_line(),
            "[jurisdiction: unknown] · [trains on you: unknown] · [open-weight: no]"
        );
    }

    #[test]
    fn eu_policy_default_stars_the_jurisdiction_chip() {
        // Mistral: EU by policy default, no-train on the paid API only, open-weight.
        let info = SovereigntyInfo {
            jurisdiction: Jurisdiction::Eu,
            residency: Residency::EuPolicyDefault,
            trains_on_you: TrainsOnYou::PaidApiOnly,
            open_weight: true,
            ..Default::default()
        };
        assert_eq!(
            info.info_line(),
            "[jurisdiction: EU*] · [trains on you: no*] · [open-weight: yes]"
        );
    }

    #[test]
    fn hard_eu_confined_has_no_star() {
        // IONOS: hard EU-confined, never trains, open-weight.
        let info = SovereigntyInfo {
            jurisdiction: Jurisdiction::Eu,
            residency: Residency::EuConfined,
            trains_on_you: TrainsOnYou::No,
            open_weight: true,
            ..Default::default()
        };
        assert_eq!(
            info.info_line(),
            "[jurisdiction: EU] · [trains on you: no] · [open-weight: yes]"
        );
    }

    #[test]
    fn honesty_flags_append_chips() {
        // An aggregator that also gates its no-train claim behind a ZDR toggle.
        let info = SovereigntyInfo {
            jurisdiction: Jurisdiction::Us,
            trains_on_you: TrainsOnYou::PaidApiOnly,
            aggregator_hop: true,
            zdr_gated: true,
            ..Default::default()
        };
        assert_eq!(
            info.info_line(),
            "[jurisdiction: US] · [trains on you: no*] · [open-weight: no] · [aggregator-hop] · [ZDR-gated]"
        );
    }

    #[test]
    fn local_is_the_most_sovereign_tier() {
        // A local runtime (Ollama): data never leaves the device, open-weight.
        let info = SovereigntyInfo {
            jurisdiction: Jurisdiction::Local,
            trains_on_you: TrainsOnYou::No,
            open_weight: true,
            ..Default::default()
        };
        assert_eq!(
            info.info_line(),
            "[jurisdiction: local] · [trains on you: no] · [open-weight: yes]"
        );
    }

    #[test]
    fn curated_encodes_the_stated_facts_and_leaves_murky_ones_unknown() {
        // Mistral: EU policy default (EU*), no-train paid-only, open-weight.
        let m = curated_for("mistral").expect("curated");
        assert_eq!(m.jurisdiction, Jurisdiction::Eu);
        assert_eq!(m.residency, Residency::EuPolicyDefault);
        assert_eq!(m.trains_on_you, TrainsOnYou::PaidApiOnly);
        assert!(m.open_weight);
        assert_eq!(
            m.info_line(),
            "[jurisdiction: EU*] · [trains on you: no*] · [open-weight: yes]"
        );
        // Anthropic: US, no-train paid-only, closed-weight.
        let a = curated_for("anthropic").expect("curated");
        assert_eq!(a.jurisdiction, Jurisdiction::Us);
        assert_eq!(a.trains_on_you, TrainsOnYou::PaidApiOnly);
        assert!(!a.open_weight);
        // DeepSeek: CN + open-weight, but the training terms are murky -> Unknown.
        let d = curated_for("deepseek").expect("curated");
        assert_eq!(d.jurisdiction, Jurisdiction::Cn);
        assert_eq!(d.trains_on_you, TrainsOnYou::Unknown);
        assert!(d.open_weight);
        // OpenRouter: an aggregator, jurisdiction inherited downstream.
        assert!(curated_for("openrouter").expect("curated").aggregator_hop);
        // An un-curated provider stays an Unknown stub.
        assert!(curated_for("some-unknown-provider").is_none());
    }

    #[test]
    fn self_host_flag_renders() {
        // Aleph Alpha: self-host-required now.
        let info = SovereigntyInfo {
            jurisdiction: Jurisdiction::Eu,
            residency: Residency::EuConfined,
            trains_on_you: TrainsOnYou::No,
            open_weight: true,
            self_host_required: true,
            ..Default::default()
        };
        assert!(info.info_line().ends_with("· [self-host]"));
    }
}
