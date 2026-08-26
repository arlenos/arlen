// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
//! Every grant a shipped profile writes must be one the launcher accepts.
//!
//! A profile is the only place an app's reach is written down, and a reader
//! trusts it. So an entry the launcher silently drops is worse than a missing
//! one: the file states a capability the app does not have, and the failure
//! shows up as the app not working rather than as anything pointing here.
//!
//! Found on 26 August, which is why this exists: `apps/pdf` declares
//! `read_only = ["/home/$USER", "/run/media/$USER"]` as its ENTIRE filesystem
//! grant, and neither entry reached the launcher. `$USER` was never expanded, so
//! the path did not exist; and the home tree is refused by the whole-tree rule
//! even once expanded. A reader that could not read, with a profile that said it
//! could.
//!
//! This runs the real resolution rather than a copy of it, so it cannot drift
//! from what the launcher does.

use std::path::{Path, PathBuf};

use arlen_permissions::{expand_user, is_host_escape, load_profile_from, read_only_grant_ok};

/// `(app_id, entry)` pairs known to be dropped, with the reason. MAY SHRINK, MAY
/// NOT GROW: a new line here is a profile that started claiming reach its app
/// does not get.
const PENDING: &[(&str, &str, &str)] = &[(
    "dev.arlen.pdf",
    "/home/$USER",
    "the whole-tree rule refuses the home tree, and read_only is this app's entire \
     filesystem grant, so a confined reader opens a PDF on a stick and nothing in \
     your home. Whether read_only may name the home tree - it stays refused for \
     custom, which is read-write - is the open decision; the alternative is \
     home = true, which is read-write over everything.",
)];

fn profiles_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../dev/mkosi/mkosi.extra/var/lib/arlen/permissions/1000")
}

#[test]
fn every_read_only_grant_a_shipped_profile_writes_is_one_the_launcher_accepts() {
    let home = Path::new("/home/u");
    let dir = profiles_dir();
    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    assert!(
        !entries.is_empty(),
        "NOTHING WAS READ: no profile under {}",
        dir.display()
    );

    let mut checked = 0usize;
    let mut dropped: Vec<String> = Vec::new();
    for path in entries {
        let app_id = path.file_stem().unwrap().to_string_lossy().into_owned();
        let profile = match load_profile_from(&path, &app_id) {
            Ok(p) => p,
            // Parsing is `check-app-profiles`' job; a parse failure there is a
            // different finding and this test should not double-report it.
            Err(_) => continue,
        };
        for entry in &profile.filesystem.custom {
            checked += 1;
            let expanded = expand_user(entry, home);
            if !expanded.is_absolute() {
                dropped.push(format!(
                    "{app_id}: custom `{}` is not absolute after expansion, so the \
                     launcher drops it - `$USER` is the only token the grammar knows",
                    entry.display()
                ));
            } else if is_host_escape(&expanded, home) {
                dropped.push(format!(
                    "{app_id}: custom `{}` resolves to `{}`, which the launcher \
                     refuses as a host escape",
                    entry.display(),
                    expanded.display()
                ));
            }
        }
        for entry in &profile.filesystem.read_only {
            checked += 1;
            let expanded = expand_user(entry, home);
            let entry_text = entry.display().to_string();
            if PENDING
                .iter()
                .any(|(a, e, _)| *a == app_id && *e == entry_text)
            {
                continue;
            }
            if !read_only_grant_ok(&expanded, home) {
                dropped.push(format!(
                    "{app_id}: read_only `{}` resolves to `{}`, which the launcher \
                     refuses, so the app is handed nothing for it",
                    entry.display(),
                    expanded.display()
                ));
            }
        }
    }

    assert!(
        checked > 0,
        "NOTHING WAS READ: no shipped profile declares a read_only grant, so this \
         test compared nothing"
    );
    assert!(
        dropped.is_empty(),
        "{} grant(s) written in a profile and dropped by the launcher:\n  - {}",
        dropped.len(),
        dropped.join("\n  - ")
    );
}
