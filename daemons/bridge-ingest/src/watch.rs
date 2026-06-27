//! The Obsidian vault floor: a one-shot sync plus a live file-watch that drive
//! the ingest sink directly (foreign-app-bridges.md, the daemon-side floor
//! reader). A watched vault's `.md` notes become `md.obsidian.Note` upserts.
//!
//! This is distinct from the generic plugin transport ([`crate::serve`] over
//! stdin): it has no foreign peer, the daemon itself reads the vault. Both reach
//! the same [`interpret_message`] -> [`PlanSink`] path, so a note ingested by the
//! floor and one ingested by the (richer) plugin tier land identically. The floor
//! is one-way ingest: a note delete is NOT propagated as a retract (close-never-
//! delete and the resolved link graph are the plugin tier's job).

use std::path::Path;

use serde_json::{Map, Value};

use crate::bridge::BridgeConfig;
use crate::host::PlanSink;
use crate::interpret::interpret_message;
use crate::obsidian;

/// The inbound message type the vault floor emits. Matches the `[map.note]` rule
/// the Obsidian `bridge.toml` declares.
const NOTE_MSG_TYPE: &str = "note";

/// Interpret one floor message against the bridge map and write the resulting
/// plan. Returns whether it was written. A mapping or write failure is logged and
/// counted as not-written, never fatal: one bad note must not abort a sync or
/// kill the watch loop. The bridge id is the config's `allowed_plugin_id` (the
/// namespace owner), the same id [`crate::serve`] writes under.
fn ingest_message<S: PlanSink>(config: &BridgeConfig, sink: &mut S, msg: &Map<String, Value>) -> bool {
    let rel = msg.get("path").and_then(Value::as_str).unwrap_or("");
    match interpret_message(config, NOTE_MSG_TYPE, msg) {
        Ok(plan) => match sink.write_plan(&config.bridge.allowed_plugin_id, &plan) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(note = rel, "obsidian floor: write failed: {e}");
                false
            }
        },
        Err(e) => {
            tracing::warn!(note = rel, "obsidian floor: not accepted by the bridge map: {e}");
            false
        }
    }
}

/// Ingest one note from its vault-relative path and content: build the floor
/// message ([`obsidian::note_message`]) and write it. Returns whether it landed.
pub fn ingest_note<S: PlanSink>(
    config: &BridgeConfig,
    sink: &mut S,
    rel_path: &str,
    content: &str,
) -> bool {
    let msg = obsidian::note_message(rel_path, content);
    ingest_message(config, sink, &msg)
}

/// One-shot full vault sync: read every `.md` note under `root` and ingest it.
/// Returns the count written. The upsert keys on the note's vault-relative path,
/// so a re-sync strengthens a note rather than duplicating it (idempotent).
/// Errors only if `root` itself is unreadable; an individual unreadable note is
/// skipped by [`obsidian::scan_vault`].
pub fn sync_vault<S: PlanSink>(
    root: &Path,
    config: &BridgeConfig,
    sink: &mut S,
) -> std::io::Result<usize> {
    let notes = obsidian::scan_vault(root)?;
    let written = notes
        .iter()
        .filter(|msg| ingest_message(config, sink, msg))
        .count();
    Ok(written)
}

/// Watch a vault for live `.md` changes and ingest each into the sink, after an
/// initial full [`sync_vault`]. Blocks until the watcher's event channel closes.
/// A create or modify of a `.md` note re-ingests it (the idempotent upsert keys
/// on its path); a hidden path (under `.obsidian` etc.) and a non-markdown file
/// are ignored, mirroring the sync filter. A removed or transiently-unreadable
/// file is skipped (the floor does not retract). Needs the `notify` watcher.
pub fn watch_vault<S: PlanSink>(
    root: &Path,
    config: &BridgeConfig,
    sink: &mut S,
) -> Result<(), String> {
    use notify::{RecursiveMode, Watcher};

    let n = sync_vault(root, config, sink).map_err(|e| format!("initial vault sync: {e}"))?;
    tracing::info!(vault = %root.display(), notes = n, "obsidian floor: initial sync done");

    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |res| {
        // A send failure means the receiver was dropped (we are shutting down).
        let _ = tx.send(res);
    })
    .map_err(|e| format!("build watcher: {e}"))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| format!("watch {}: {e}", root.display()))?;

    for res in rx {
        let event = match res {
            Ok(event) => event,
            Err(e) => {
                tracing::warn!("obsidian floor: watch error: {e}");
                continue;
            }
        };
        for path in event.paths {
            let Some(rel) = obsidian::vault_relative_md(root, &path) else {
                continue;
            };
            // A delete or an editor's atomic-rename can leave the path gone by the
            // time we read it; skip it rather than treating absence as a retract.
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    ingest_note(config, sink, &rel, &content);
                }
                Err(e) => tracing::debug!(note = %rel, "obsidian floor: skip unreadable change: {e}"),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::BridgeConfig;
    use crate::interpret::UpsertPlan;

    /// A sink that records each written plan, for asserting what a sync produced.
    #[derive(Default)]
    struct RecordingSink {
        plans: Vec<UpsertPlan>,
    }
    impl PlanSink for RecordingSink {
        fn write_plan(&mut self, _bridge: &str, plan: &UpsertPlan) -> Result<(), String> {
            self.plans.push(plan.clone());
            Ok(())
        }
    }

    fn obsidian_config() -> BridgeConfig {
        // The shipped Obsidian floor mapping: a `note` becomes an md.obsidian.Note
        // keyed by path, projecting title/tags/links.
        BridgeConfig::parse(
            "[bridge]\nallowed_plugin_id = \"md.obsidian.arlen-bridge\"\n\
             [map.note]\nupsert = \"md.obsidian.Note\"\nkey = \"path\"\n\
             set = { title = \"$.title\", tags = \"$.tags\", links = \"$.links\" }\n",
        )
        .expect("obsidian bridge config parses")
    }

    #[test]
    fn sync_vault_ingests_each_markdown_note_keyed_by_path() {
        let dir = std::env::temp_dir().join(format!("arlen-obsidian-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".obsidian")).unwrap();
        std::fs::write(dir.join("Top.md"), "# Top with [[Other]]\n#idea\n").unwrap();
        std::fs::write(dir.join("sub").join("Nested.md"), "---\ntitle: Nested\n---\nbody\n").unwrap();
        std::fs::write(dir.join("notes.txt"), "not markdown").unwrap();
        std::fs::write(dir.join(".obsidian").join("hidden.md"), "#skip\n").unwrap();

        let mut sink = RecordingSink::default();
        let n = sync_vault(&dir, &obsidian_config(), &mut sink).unwrap();

        assert_eq!(n, 2, "two real notes ingested, the .txt and the hidden one skipped");
        let keys: Vec<&str> = sink.plans.iter().map(|p| p.external_key.as_str()).collect();
        assert!(keys.contains(&"Top.md"));
        assert!(keys.contains(&"sub/Nested.md"));
        assert!(sink.plans.iter().all(|p| p.qualified_type == "md.obsidian.Note"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ingest_note_writes_one_keyed_plan() {
        let mut sink = RecordingSink::default();
        let landed = ingest_note(&obsidian_config(), &mut sink, "Daily/2026-06-27.md", "#journal\n");
        assert!(landed);
        assert_eq!(sink.plans.len(), 1);
        assert_eq!(sink.plans[0].external_key, "Daily/2026-06-27.md");
        assert_eq!(sink.plans[0].qualified_type, "md.obsidian.Note");
    }
}
