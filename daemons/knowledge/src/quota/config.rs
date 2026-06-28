/// Quota configuration: tier definitions, per-app overrides.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

/// Application tier determining default quota limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppTier {
    System,
    #[serde(alias = "first-party")]
    FirstParty,
    #[serde(alias = "third-party")]
    ThirdParty,
}

/// Resource limits for a tier or specific app.
#[derive(Debug, Clone, Deserialize)]
pub struct TierQuotas {
    /// Max entities (None = unlimited).
    pub max_entities: Option<usize>,
    /// Max storage in bytes (None = unlimited).
    pub max_storage_bytes: Option<usize>,
    /// Max rows returned per query.
    pub max_query_results: usize,
    /// Max queries per second (None = unlimited).
    pub queries_per_second: Option<usize>,
    /// Max writes per second (None = unlimited).
    pub writes_per_second: Option<usize>,
}

impl TierQuotas {
    pub fn system() -> Self {
        Self {
            max_entities: None,
            max_storage_bytes: None,
            max_query_results: 100_000,
            queries_per_second: None,
            writes_per_second: None,
        }
    }

    pub fn first_party() -> Self {
        Self {
            max_entities: Some(500_000),
            max_storage_bytes: Some(2 * 1024 * 1024 * 1024),
            max_query_results: 50_000,
            queries_per_second: Some(1000),
            writes_per_second: Some(500),
        }
    }

    pub fn third_party() -> Self {
        Self {
            max_entities: Some(100_000),
            max_storage_bytes: Some(500 * 1024 * 1024),
            max_query_results: 10_000,
            queries_per_second: Some(100),
            writes_per_second: Some(50),
        }
    }

    pub fn for_tier(tier: AppTier) -> Self {
        match tier {
            AppTier::System => Self::system(),
            AppTier::FirstParty => Self::first_party(),
            AppTier::ThirdParty => Self::third_party(),
        }
    }
}

/// Top-level quota configuration file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct QuotaConfig {
    /// App IDs that get first-party limits.
    #[serde(default)]
    pub first_party_apps: Vec<String>,
    /// Per-app quota overrides.
    #[serde(default)]
    pub overrides: HashMap<String, TierQuotas>,
}

impl QuotaConfig {
    /// Load from TOML file (returns the Arlen default if missing).
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        if !path.exists() {
            return Ok(Self::arlen_default());
        }
        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// The default config the live query daemon uses when no
    /// `graph.toml` quota section is present.
    ///
    /// The canonical identity resolver yields bare ids for system
    /// binaries (`ai-daemon`, `ai-agent`) rather than `org.arlen.*`,
    /// so they must be named here to receive the AI layer's higher
    /// (FirstParty, 1000 qps) limit — foundation §8.4: "the AI daemon
    /// has a higher sustained limit." Without this they would fall to
    /// ThirdParty (100 qps) and be wrongly throttled. Genuine
    /// provenance-based tiering (canonical /usr install vs a
    /// user-installed lookalike of the same id) shares the F3 identity
    /// gap's future fix (the installd inode registry); a `graph.toml`
    /// may extend `first_party_apps` for other first-party apps.
    /// `code-indexer` is likewise a first-party system daemon: it reads the live
    /// `Project` roots (`MATCH (p:Project) RETURN p.root_path`) to scope which
    /// files it indexes, then emits `code.indexed`. Without FirstParty it tiers
    /// ThirdParty, so that Project read is refused by the read-scope label gate
    /// and the daemon indexes nothing at all (it gates indexing on being able to
    /// read the project roots) - inert in a real deployment.
    pub fn arlen_default() -> Self {
        Self {
            first_party_apps: vec![
                "ai-daemon".to_string(),
                "ai-agent".to_string(),
                "code-indexer".to_string(),
            ],
            overrides: HashMap::new(),
        }
    }

    // (debug only) cargo-run id of a first-party daemon. A locally-built
    // `ai-agent`/`ai-daemon`/`code-indexer` resolves via `path_to_app_id` rule (5)
    // to `dev.arlen-{name}`, not the canonical `ai-agent`/`code-indexer`, so it
    // would otherwise tier as ThirdParty: the dev agent's graph writes would be
    // refused, and the dev code-indexer's `MATCH (p:Project)` read denied (leaving
    // it inert) in `just dev` and the integration harness. This admits exactly
    // those ids as FirstParty in debug builds — the analog of the audit daemon's
    // `dev.*` ADMITTED allowance — so the assembled stack can be exercised against
    // a dev/test graph. Compiled out of release (`debug_assertions`), where only
    // the canonical attested ids tier FirstParty.
    #[cfg(debug_assertions)]
    fn is_dev_first_party_id_impl(app_id: &str) -> bool {
        matches!(
            app_id,
            "dev.arlen-ai-agent" | "dev.arlen-ai-daemon" | "dev.arlen-code-indexer"
        )
    }
    #[cfg(not(debug_assertions))]
    fn is_dev_first_party_id_impl(_app_id: &str) -> bool {
        false
    }

    /// Determine the tier for an app.
    pub fn tier_for_app(&self, app_id: &str) -> AppTier {
        if app_id == "system" || app_id.starts_with("system.") {
            AppTier::System
        } else if self.first_party_apps.contains(&app_id.to_string())
            || app_id.starts_with("org.arlen.")
            || Self::is_dev_first_party_id_impl(app_id)
        {
            AppTier::FirstParty
        } else {
            AppTier::ThirdParty
        }
    }

    /// Get the effective quotas for an app (override or tier default).
    pub fn quotas_for_app(&self, app_id: &str) -> TierQuotas {
        if let Some(q) = self.overrides.get(app_id) {
            return q.clone();
        }
        TierQuotas::for_tier(self.tier_for_app(app_id))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> QuotaConfig {
        QuotaConfig {
            first_party_apps: vec!["org.arlen.contacts".into(), "com.partner".into()],
            overrides: HashMap::new(),
        }
    }

    #[test]
    fn test_tier_system() {
        let c = config();
        assert_eq!(c.tier_for_app("system"), AppTier::System);
        assert_eq!(c.tier_for_app("system.daemon"), AppTier::System);
    }

    #[test]
    fn test_tier_first_party() {
        let c = config();
        assert_eq!(c.tier_for_app("org.arlen.contacts"), AppTier::FirstParty);
        assert_eq!(c.tier_for_app("org.arlen.calendar"), AppTier::FirstParty);
        assert_eq!(c.tier_for_app("com.partner"), AppTier::FirstParty);
    }

    #[test]
    fn test_code_indexer_is_first_party_by_default() {
        // code-indexer reads the live Project roots to scope its indexing; it must
        // tier FirstParty (system-anchored) in the shipped default or that read is
        // refused and the daemon indexes nothing. Regression guard for the inert
        // sensor the assembled-stack run surfaced.
        let c = QuotaConfig::arlen_default();
        assert_eq!(c.tier_for_app("code-indexer"), AppTier::FirstParty);
    }

    #[test]
    fn test_tier_third_party() {
        let c = config();
        assert_eq!(c.tier_for_app("com.anki"), AppTier::ThirdParty);
        assert_eq!(c.tier_for_app("org.zotero"), AppTier::ThirdParty);
    }

    #[test]
    fn test_tier_dev_first_party_id_is_first_party_in_debug() {
        // Tests run under debug_assertions, so the dev cargo-run ids of the
        // first-party daemons tier FirstParty here (the analog of the audit
        // daemon admitting `dev.*` producers in debug), letting the assembled
        // stack run against a dev graph. An unlisted `dev.*` id stays ThirdParty.
        let c = QuotaConfig::arlen_default();
        assert_eq!(c.tier_for_app("dev.arlen-ai-agent"), AppTier::FirstParty);
        assert_eq!(c.tier_for_app("dev.arlen-ai-daemon"), AppTier::FirstParty);
        assert_eq!(c.tier_for_app("dev.arlen-code-indexer"), AppTier::FirstParty);
        assert_eq!(c.tier_for_app("dev.arlen-knowledge"), AppTier::ThirdParty);
        assert_eq!(c.tier_for_app("dev.evil"), AppTier::ThirdParty);
    }

    #[test]
    fn test_quotas_for_tier() {
        let sys = TierQuotas::system();
        assert!(sys.max_entities.is_none());
        assert!(sys.queries_per_second.is_none());

        let fp = TierQuotas::first_party();
        assert_eq!(fp.max_entities, Some(500_000));
        assert_eq!(fp.queries_per_second, Some(1000));

        let tp = TierQuotas::third_party();
        assert_eq!(tp.max_entities, Some(100_000));
        assert_eq!(tp.queries_per_second, Some(100));
    }

    #[test]
    fn test_per_app_override() {
        let mut c = config();
        c.overrides.insert(
            "com.special".into(),
            TierQuotas {
                max_entities: Some(999),
                max_storage_bytes: Some(1024),
                max_query_results: 50,
                queries_per_second: Some(10),
                writes_per_second: Some(5),
            },
        );
        let q = c.quotas_for_app("com.special");
        assert_eq!(q.max_entities, Some(999));
        assert_eq!(q.max_query_results, 50);
    }

    #[test]
    fn test_quotas_for_app_default() {
        let c = config();
        let q = c.quotas_for_app("com.anki");
        assert_eq!(q.max_entities, Some(100_000));
        assert_eq!(q.max_query_results, 10_000);
    }
}
