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
