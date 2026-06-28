//! `arlen-kg-seed` - write the deterministic dev KG corpus
//! ([`knowledge::seed`]) into the graph store at `ARLEN_GRAPH_PATH`.
//!
//! Dev/test verification only: it REQUIRES `ARLEN_GRAPH_PATH` (no default), so
//! it can never accidentally seed a production store at `/var/lib` or
//! `~/.local/share`. Run with the knowledge daemon STOPPED (ladybug/Kuzu is
//! single-writer); the daemon then serves arlen-ui's KG surfaces from the
//! seeded store. Idempotent - safe to re-run.

use anyhow::{anyhow, Context, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let path = std::env::var("ARLEN_GRAPH_PATH").map_err(|_| {
        anyhow!(
            "ARLEN_GRAPH_PATH must be set (dev/test only; refusing to guess a \
             store path so the real graph is never touched)"
        )
    })?;

    // Kuzu opens the store as a directory; make sure its parent exists.
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create graph parent dir {}", parent.display()))?;
    }

    let graph = knowledge::graph::spawn(&path)?;
    knowledge::seed::seed_corpus(&graph).await?;
    println!("arlen-kg-seed: seeded the deterministic dev KG corpus into {path}");
    Ok(())
}
