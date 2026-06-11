//! The Arlen online-accounts daemon (`org.arlen.Accounts1`).
//!
//! Skeleton (OA-R1, this slice): resolve and load the account configs, surfacing
//! malformed ones rather than silently granting them, and prepare the capability
//! gate. The D-Bus ObjectManager + per-service interfaces, the SO_PEERCRED +
//! `path_to_app_id` caller-auth at every method boundary, and the Secret Service
//! token handout are the next slice (online-accounts-plan.md OA-R1).

use online_accounts::config;
use online_accounts::gate::AccessGate;

fn main() {
    let Some(dir) = config::accounts_dir() else {
        eprintln!("arlen-accountsd: no config home; nothing to serve");
        return;
    };
    let (accounts, errors) = config::load_accounts(&dir);
    for (path, err) in &errors {
        // A malformed config is reported and skipped, never granted.
        eprintln!("arlen-accountsd: skipping {}: {err}", path.display());
    }
    // The gate is ready over the loaded set; the D-Bus surface that resolves the
    // caller and consults it is the next slice.
    let _gate = AccessGate::new(&accounts);
    eprintln!(
        "arlen-accountsd: loaded {} account(s) from {}; D-Bus serve not yet wired (OA-R1 next slice)",
        accounts.len(),
        dir.display()
    );
}
