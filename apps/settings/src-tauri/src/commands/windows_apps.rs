//! Windows-apps commands (`windows-apps-plan.md`): the bottles this machine
//! holds, read from the bottle daemon over its socket.
//!
//! WHAT THIS CAN ANSWER, and it is deliberately less than the panel renders. A
//! bottle knows its id, whether it may reach the network, and which host folders
//! it was granted as drive letters. It does not know its Wine version, its DLL
//! overrides, its winetricks verbs, DXVK, scaling or a window mode: those come
//! from the compat recipe, which the plan lists as its own piece and which does
//! not exist yet. Filling them in here would be inventing them, and a switch
//! drawn from an invented value writes to a bottle that does not hold it.
//!
//! `ask` is synchronous one-shot-per-connection, so each call runs on a blocking
//! thread to keep the async runtime free - the same shape as the capsule commands.

use arlen_wine_core::client::ask;
use arlen_wine_core::protocol::{Request, Response};
use arlen_wine_core::server::socket_path;
use serde::Serialize;
use tauri_plugin_arlen_portal::{FileFilter, FilterPattern, PickFileOptions, PickerResult};

/// One bottle as a panel row, as much of it as the runtime actually knows.
///
/// Named `BottleRow` rather than `Bottle` on purpose: the runtime has a `Bottle`
/// of its own with a different shape, and two types one word apart in one
/// codebase is how a reader - and a checker - resolves the wrong one.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BottleRow {
    pub id: String,
    /// Whether the app in this bottle may reach the network at all.
    pub network: bool,
    /// Whether one of its granted drives is the person's home folder.
    pub home_folder: bool,
    /// The drives it was granted: the letter a Windows program sees and the host
    /// folder behind it.
    pub drives: Vec<DriveRow>,
    /// Whether somebody has said which program this bottle starts. False after an
    /// install until they pick, and the panel has a question to ask while it is.
    pub has_program: bool,
}

/// One granted drive, as `windows-apps.ts` declares it. `DriveRow` for the same
/// reason as `BottleRow`: the runtime's own `Drive` carries more.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveRow {
    pub letter: String,
    pub path: String,
}

/// What a listing came back with.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bottles {
    pub bottles: Vec<BottleRow>,
    /// Bottles that are on disk and did not read.
    ///
    /// SEPARATE from the list, because "you have no bottles" and "one of your
    /// bottles is broken" are different sentences and the second one is the one
    /// somebody can act on.
    pub unreadable: Vec<String>,
}

/// The bottles on this machine.
///
/// An error is the daemon not being reachable, which the panel already renders as
/// unreachable rather than as an empty machine.
#[tauri::command]
pub async fn list_bottles() -> Result<Bottles, String> {
    tokio::task::spawn_blocking(|| match ask(&socket_path(), &Request::ListBottles) {
        Ok(Response::Bottles {
            bottles,
            unreadable,
        }) => Ok(Bottles {
            bottles: bottles
                .into_iter()
                .map(|b| BottleRow {
                    id: b.id,
                    network: b.network,
                    home_folder: b.home_folder,
                    has_program: b.has_program,
                    drives: b
                        .drives
                        .into_iter()
                        .map(|d| DriveRow {
                            letter: d.letter.to_string(),
                            path: d.path,
                        })
                        .collect(),
                })
                .collect(),
            unreadable,
        }),
        // A refusal to a list ask, or an answer to a question nobody asked, is a
        // daemon this build does not agree with - surfaced rather than smoothed
        // into an empty list.
        Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What one bottle's prefix says, against what the bottle says it is.
///
/// The two fields the panel reads: whether the two agree, and how many links
/// leave the prefix without a grant behind them. An error is "could not check",
/// which the store already keeps apart from "healthy" - a green light nobody
/// measured is the claim this whole surface exists to avoid.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BottleHealth {
    pub agrees: bool,
    pub escapes: usize,
}

/// Check one bottle against its prefix.
#[tauri::command]
pub async fn bottle_health(id: String) -> Result<BottleHealth, String> {
    tokio::task::spawn_blocking(move || {
        match ask(&socket_path(), &Request::Health { id }) {
            Ok(Response::Health {
                agrees, escapes, ..
            }) => Ok(BottleHealth { agrees, escapes }),
            // A refusal names which one, so the window can tell "no such bottle"
            // from "it is there and unreadable" rather than saying one for both.
            Ok(Response::Refused { problem }) => Err(format!("{problem:?}")),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Start the Windows program a bottle exists to run.
///
/// The daemon owns the process, so it outlives this window - which is the reason
/// the runtime is a daemon at all. What starts is what the bottle records, not
/// something this command names, so the panel cannot ask for a different program
/// inside somebody's confinement.
///
/// A refusal comes back as its own token, because the four reasons need four
/// different sentences: nothing is installed in this bottle yet, this machine has
/// no Wine, the drive table promises reach the confinement does not give, or the
/// confinement would not start.
#[tauri::command]
pub async fn launch_windows_app(id: String) -> Result<u32, String> {
    tokio::task::spawn_blocking(move || match ask(&socket_path(), &Request::Launch { id }) {
        Ok(Response::Launched { pid }) => Ok(pid),
        // The token, not its Debug form: `nothing-to-run` is what the daemon
        // speaks on the wire and what the window matches on to pick a sentence.
        // Five reasons that arrive as one string are one sentence, and four of
        // them would be wrong.
        Ok(Response::Refused { problem }) => Err(problem_token(problem)),
        Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// A refusal as its wire token (`nothing-to-run`, `no-wine`, ...).
fn problem_token(problem: arlen_wine_core::protocol::Problem) -> String {
    serde_json::to_value(problem)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        // A token that will not serialise is not a sentence anybody should read,
        // so it falls back to the one the window already has for "it did not
        // start and nobody can say why".
        .unwrap_or_else(|| "could-not-start".to_string())
}

/// Forget a bottle: its prefix goes to the trash and its description is removed.
///
/// Answers with WHERE the prefix went, so the panel can say it rather than imply
/// it. `None` means there was no prefix on disk to move.
///
/// The runtime admits only this app for this ask and records it in the ledger
/// before it happens, so a refusal here can also mean the ledger was unreachable -
/// which is the fail-closed answer, not a bug to route around.
#[tauri::command]
pub async fn delete_bottle(id: String) -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(move || match ask(&socket_path(), &Request::Forget { id }) {
        Ok(Response::Forgotten { trashed_to }) => Ok(trashed_to),
        Ok(Response::Refused { problem }) => Err(problem_token(problem)),
        Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What this machine can run Windows programs with.
///
/// `None` means there is no Wine here, which is a fact somebody needs before they
/// wonder why nothing starts. The panel used to open with a list of runtimes as
/// though they were installed; this is the same list, measured.
#[tauri::command]
pub async fn windows_runtimes() -> Result<Option<String>, String> {
    tokio::task::spawn_blocking(|| match ask(&socket_path(), &Request::Runtimes) {
        Ok(Response::Runtimes { wine }) => Ok(wine),
        Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Open a bottle's C: drive in the file manager.
///
/// THE PATH COMES FROM THE DAEMON, never from the caller. This command hands over
/// a bottle id and opens whatever the daemon answers with, so a window cannot use
/// it to open an arbitrary folder: the id is validated against the bottle
/// registry, and a bottle that was never booted has no C: drive and is refused by
/// name rather than opened on nothing.
///
/// Opening goes through the shared portal helper, the same route `open_url` takes,
/// so a folder from Settings lands in whatever file manager the session has.
#[tauri::command]
pub async fn browse_bottle_files(id: String) -> Result<(), String> {
    let path = tokio::task::spawn_blocking(move || {
        match ask(&socket_path(), &Request::Prefix { id }) {
            Ok(Response::Prefix { path }) => Ok(path),
            Ok(Response::Refused { problem }) => Err(problem_token(problem)),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    tauri_plugin_arlen_portal::api::open_external(&format!("file://{path}"))
        .await
        .map_err(|e| e.to_string())
}

/// What a cache sweep removed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearedCaches {
    pub bytes: u64,
    pub files: usize,
}

/// Clear a bottle's regenerable caches.
///
/// The number that comes back is what was ACTUALLY removed, zero included: a
/// bottle with nothing to clear should say so rather than let a surface imply it
/// reclaimed something. What counts as regenerable, and why the daemon's sweep
/// never follows a link out of the prefix, is in `caches.rs` - the short version
/// is that a Wine prefix is full of links into the person's home.
#[tauri::command]
pub async fn clear_bottle_caches(id: String) -> Result<ClearedCaches, String> {
    tokio::task::spawn_blocking(move || {
        match ask(&socket_path(), &Request::ClearCaches { id }) {
            Ok(Response::Cleared { bytes, files }) => Ok(ClearedCaches { bytes, files }),
            Ok(Response::Refused { problem }) => Err(problem_token(problem)),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One program an installer left inside a bottle.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramRow {
    pub path: String,
    pub name: String,
}

/// Install a Windows app: pick an installer, make a bottle for it, run it there.
///
/// Three daemon asks rather than one, because they fail differently and the person
/// needs to know which happened: the bottle could not be made, or it was made and
/// the installer would not start. The bottle survives the second, which is what
/// lets them try again without losing the prefix.
///
/// Answers the new bottle's id, or `None` when the picker was cancelled - a
/// cancel is not an error and must not raise one.
///
/// WHAT THIS DOES NOT DO is decide which program the installer installed. That is
/// `bottle_programs` and `set_bottle_program`, because an installer leaves several
/// (the app, an uninstaller, a crash reporter) and picking automatically is a
/// guess a person pays for later.
#[tauri::command]
pub async fn install_windows_app() -> Result<Option<String>, String> {
    let picked = tauri_plugin_arlen_portal::api::pick_file(PickFileOptions {
        title: Some("Choose a Windows installer".into()),
        filters: vec![FileFilter {
            name: "Windows installers".into(),
            patterns: vec![
                FilterPattern::Glob {
                    pattern: "*.exe".into(),
                },
                FilterPattern::Glob {
                    pattern: "*.msi".into(),
                },
            ],
        }],
        ..Default::default()
    })
    .await
    .map_err(|e| e.to_string())?;

    let uri = match picked {
        PickerResult::Picked { uris } => match uris.into_iter().next() {
            Some(u) => u,
            // Picked nothing is the same as cancelling, and neither is a failure.
            None => return Ok(None),
        },
        PickerResult::Cancelled => return Ok(None),
    };
    // The portal answers with a URI; the daemon takes a path.
    let path = uri.strip_prefix("file://").unwrap_or(&uri).to_string();
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    let id = arlen_wine_core::install::id_from_installer(&name)
        .ok_or_else(|| "unnamed-installer".to_string())?;

    let made = id.clone();
    tokio::task::spawn_blocking(move || {
        match ask(&socket_path(), &Request::Create { id: made.clone() }) {
            Ok(Response::Created { .. }) => Ok(()),
            Ok(Response::Refused { problem }) => Err(problem_token(problem)),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    let into = id.clone();
    tokio::task::spawn_blocking(move || {
        match ask(
            &socket_path(),
            &Request::Install {
                id: into,
                installer: path,
            },
        ) {
            Ok(Response::Launched { .. }) => Ok(()),
            Ok(Response::Refused { problem }) => Err(problem_token(problem)),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(Some(id))
}

/// The programs found inside a bottle, for somebody to pick the app from.
#[tauri::command]
pub async fn bottle_programs(id: String) -> Result<Vec<ProgramRow>, String> {
    tokio::task::spawn_blocking(move || {
        match ask(&socket_path(), &Request::Programs { id }) {
            Ok(Response::Programs { programs }) => Ok(programs
                .into_iter()
                .map(|p| ProgramRow {
                    path: p.path,
                    name: p.name,
                })
                .collect()),
            Ok(Response::Refused { problem }) => Err(problem_token(problem)),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Record which program a bottle's launch should start.
///
/// The daemon refuses a path that is not inside the bottle's own prefix, so this
/// cannot be used to point a bottle at something else on the machine.
#[tauri::command]
pub async fn set_bottle_program(id: String, program: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        match ask(&socket_path(), &Request::SetProgram { id, program }) {
            Ok(Response::ProgramSet { .. }) => Ok(()),
            Ok(Response::Refused { problem }) => Err(problem_token(problem)),
            Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
