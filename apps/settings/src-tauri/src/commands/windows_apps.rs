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

/// One bottle, as much of it as the runtime actually knows.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bottle {
    pub id: String,
    /// Whether the app in this bottle may reach the network at all.
    pub network: bool,
    /// Whether one of its granted drives is the person's home folder.
    pub home_folder: bool,
    /// The drives it was granted: the letter a Windows program sees and the host
    /// folder behind it.
    pub drives: Vec<Drive>,
}

/// One granted drive, as `windows-apps.ts` declares it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drive {
    pub letter: String,
    pub path: String,
}

/// What a listing came back with.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bottles {
    pub bottles: Vec<Bottle>,
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
                .map(|b| Bottle {
                    id: b.id,
                    network: b.network,
                    home_folder: b.home_folder,
                    drives: b
                        .drives
                        .into_iter()
                        .map(|d| Drive {
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
        Ok(Response::Refused { problem }) => Err(format!("{problem:?}")),
        Ok(other) => Err(format!("the Windows runtime answered {other:?}")),
        Err(e) => Err(e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())?
}
