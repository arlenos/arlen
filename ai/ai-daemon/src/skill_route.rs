//! Skill routing: hand a matched skill to the agent to run (PR-5 3c commit B).
//!
//! The interactive daemon answers most queries itself (the bounded tool loop),
//! but when a query clearly fits a loaded skill's `whenToUse` it routes the run
//! to the autonomous agent over `org.arlen.AIAgent1.run_skill`. Execution — and
//! enablement enforcement — stays in the agent (`ai-tool-routing.md`: the
//! daemon's interactive loop and the agent's autonomous loop are deliberately
//! separate). The match is enablement-aware (`skills::match_skill` only matches
//! an enabled skill), so the daemon never routes to a disabled one.
//!
//! The router is a seam so the routing decision is unit-testable with a mock,
//! without a live session bus.

use async_trait::async_trait;

/// Hands a matched skill to the agent and returns its run summary.
#[async_trait]
pub trait SkillRouter: Send + Sync {
    /// Run the named skill on the agent (`org.arlen.AIAgent1.run_skill`); the
    /// returned string is the run's status summary. `Err` carries a
    /// human-readable reason (the daemon then falls back to answering itself).
    async fn run_skill(&self, name: &str) -> Result<String, String>;
}

/// Production router: an `org.arlen.AIAgent1` proxy on the session bus.
pub struct DbusSkillRouter {
    conn: zbus::Connection,
}

impl DbusSkillRouter {
    /// Build a router over the daemon's existing session-bus connection.
    pub fn new(conn: zbus::Connection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl SkillRouter for DbusSkillRouter {
    async fn run_skill(&self, name: &str) -> Result<String, String> {
        let proxy = zbus::Proxy::new(
            &self.conn,
            "org.arlen.AIAgent1",
            "/org/arlen/AIAgent1",
            "org.arlen.AIAgent1",
        )
        .await
        .map_err(|e| format!("agent proxy: {e}"))?;
        proxy
            .call("run_skill", &(name,))
            .await
            .map_err(|e| format!("run_skill call: {e}"))
    }
}
