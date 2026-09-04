//! The open-a-Windows-file prompt (`windows-apps-plan.md`).
//!
//! Double-clicking a `.exe` is the on-ramp the plan names, and it is a TRUST
//! moment rather than a file type nobody configured. So the shell's launch
//! service does not look for a handler for one: it raises this prompt, which
//! says what the file is, how well it is known to work, what a fresh bottle
//! would grant it, and offers Run against Install.
//!
//! WHAT THE PROMPT IS ALLOWED TO CLAIM. Every field here is measured or
//! honestly absent. The compatibility tier is `untested` for everything, because
//! no compatibility database exists on this machine yet and "verified" would be
//! a promise nothing checked. The access list is what a fresh bottle actually
//! grants - its own files, nothing else - rather than what a recipe might ask
//! for. There is no recipe and no runtime fetch to report, so neither is
//! reported.

use std::sync::{Arc, Mutex};

use serde::Serialize;

/// A `.msi` is an installer; a `.exe` may be either and is not guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Installer,
    Portable,
}

/// The Windows file waiting on a decision, in the shape the dialog declares.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingWindowsFile {
    pub id: u64,
    /// Best-effort app name: the file's own name without its extension, which is
    /// as much as this knows. Reading a name out of the executable's version
    /// resource is a real improvement and a separate piece of work.
    pub app_name: String,
    pub file_name: String,
    pub file_kind: FileKind,
    /// Always `untested`: no compatibility database exists here, and a tier is a
    /// claim about whether somebody's program will work.
    pub tier: String,
    /// What a fresh bottle grants. Its own files and nothing else, which is what
    /// `arlen-bottled` actually creates.
    pub access: Vec<String>,
}

/// The path behind a pending prompt, kept out of the payload.
///
/// The dialog is told a file NAME and acts by id; the full path stays here. It
/// is what the run and install asks need and not something the window has any
/// use for, and a modal that outlives the file window should not be carrying the
/// directory somebody keeps their downloads in.
pub struct Pending {
    pub request: PendingWindowsFile,
    pub path: String,
}

/// The prompt's slot: at most one waiting file.
///
/// One, not a queue. Two `.exe` files opened at once is not a thing that happens
/// by accident, and a queue of trust decisions is a thing somebody clicks
/// through rather than reads. A second open replaces the first, which loses
/// nothing: neither has been acted on.
pub type PendingSlot = Arc<Mutex<Option<Pending>>>;

/// A fresh empty slot.
#[must_use]
pub fn new_slot() -> PendingSlot {
    Arc::new(Mutex::new(None))
}

/// The kind a file's name implies.
#[must_use]
pub fn kind_of(file_name: &str) -> FileKind {
    if file_name.to_ascii_lowercase().ends_with(".msi") {
        FileKind::Installer
    } else {
        FileKind::Portable
    }
}

/// The name to show for the app: the file name without its extension.
#[must_use]
pub fn app_name_of(file_name: &str) -> String {
    match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_string(),
        _ => file_name.to_string(),
    }
}

/// The bottle id a file gets: its name, reduced to something that is a directory
/// name and a bottle id at once.
///
/// Lowercase, with every character outside `[a-z0-9._-]` folded to a dash, so a
/// program called `Setup (2024)!.exe` cannot become a path with a space, a
/// bracket or a quote in it. Empty after folding falls back to a fixed name
/// rather than an empty id.
#[must_use]
pub fn bottle_id_for(file_name: &str) -> String {
    let stem = app_name_of(file_name).to_ascii_lowercase();
    let folded: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = folded.trim_matches('-').to_string();
    // A name with no letter or digit left in it is not an id, and my own test
    // caught why that matters: `...exe` folded to `..`, which is a directory
    // name that means the parent. The rule is the strong one rather than a list
    // of the two dot cases - anything that carries no alphanumeric character
    // cannot name a bottle.
    if trimmed.chars().any(|c| c.is_ascii_alphanumeric()) {
        trimmed
    } else {
        "windows-app".to_string()
    }
}

/// Put a file in front of the person, replacing whatever was waiting.
pub fn raise(slot: &PendingSlot, path: &str, next_id: &Mutex<u64>) -> Option<PendingWindowsFile> {
    let file_name = path.rsplit('/').next().unwrap_or(path).to_string();
    if file_name.is_empty() {
        return None;
    }
    let id = {
        let mut n = next_id.lock().ok()?;
        *n = n.wrapping_add(1);
        *n
    };
    let request = PendingWindowsFile {
        id,
        app_name: app_name_of(&file_name),
        file_name,
        file_kind: kind_of(path),
        tier: "untested".to_string(),
        access: vec!["Its own files".to_string()],
    };
    let mut held = slot.lock().ok()?;
    *held = Some(Pending {
        request: request.clone(),
        path: path.to_string(),
    });
    Some(request)
}

/// Take the waiting file if it is the one `id` names.
///
/// By id, so a stale dialog - one left open while another file was opened -
/// acts on nothing rather than on the file that replaced it.
pub fn take(slot: &PendingSlot, id: u64) -> Option<Pending> {
    let mut held = slot.lock().ok()?;
    match held.as_ref() {
        Some(p) if p.request.id == id => held.take(),
        _ => None,
    }
}

/// The Windows file waiting on a decision, or nothing.
#[tauri::command]
pub fn windows_file_request() -> Option<PendingWindowsFile> {
    crate::launch_service::pending_windows_file()
        .lock()
        .ok()?
        .as_ref()
        .map(|p| p.request.clone())
}

/// Put the file in a bottle of its own and start it.
///
/// ONE ASK IS TWO ASKS TO THE DAEMON, and that is the daemon's design rather
/// than a gap here: making the container and putting something in it fail
/// differently, so `arlen-bottled` splits them and a caller learns which
/// happened. Run and install take the same two steps; what differs is what is
/// left afterwards, which is the person picking a program to keep.
async fn into_a_bottle(id: u64) -> Result<(), String> {
    let taken = crate::windows_file::take(crate::launch_service::pending_windows_file(), id)
        .ok_or_else(|| "that file is no longer waiting, so nothing was started".to_string())?;
    let bottle = bottle_id_for(&taken.request.file_name);
    let path = taken.path;
    tokio::task::spawn_blocking(move || {
        use arlen_wine_core::client::ask;
        use arlen_wine_core::protocol::{Request, Response};
        use arlen_wine_core::server::socket_path;

        use arlen_wine_core::protocol::Problem;

        let socket = socket_path();
        // A bottle of that name already being there is NOT a failure: somebody
        // opening the same installer twice means it, and the second run belongs
        // in the same bottle as the first. Every other refusal is one.
        match ask(&socket, &Request::Create { id: bottle.clone() }) {
            Ok(Response::Created { .. })
            | Ok(Response::Refused {
                problem: Problem::BottleExists,
            }) => {}
            Ok(Response::Refused { problem }) => return Err(refusal(problem)),
            Ok(_) => return Err("the Windows runtime answered something else".to_string()),
            Err(e) => return Err(e.to_string()),
        }
        // Answers when the program STARTS, not when it finishes: an installer is
        // something a person clicks through, and waiting for that would hold the
        // dialog open for as long as they take to read a licence.
        match ask(
            &socket,
            &Request::Install {
                id: bottle,
                installer: path,
            },
        ) {
            Ok(Response::Launched { .. }) => Ok(()),
            Ok(Response::Refused { problem }) => Err(refusal(problem)),
            Ok(_) => Err("the Windows runtime answered something else".to_string()),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
    .map_err(|e| format!("the Windows runtime could not be asked: {e}"))?
}

/// What a refusal from the Windows runtime means, as something to show.
///
/// The daemon answers with a token rather than a sentence, deliberately - the
/// window writes the wording - and this is that window's side of it.
fn refusal(problem: arlen_wine_core::protocol::Problem) -> String {
    use arlen_wine_core::protocol::Problem as P;
    match problem {
        P::NoWine => "Wine is not installed, so there is nothing to run this with.".to_string(),
        P::BadId => "That file's name cannot be used for a Windows app.".to_string(),
        P::NoSuchBottle => "That Windows app is not set up on this machine.".to_string(),
        P::CouldNotCreate => "The Windows app could not be set up.".to_string(),
        other => format!("The Windows runtime refused this ({other:?})."),
    }
}

/// Run it as a one-off.
#[tauri::command]
pub async fn windows_file_run(id: u64) -> Result<(), String> {
    into_a_bottle(id).await
}

/// Install it as an app of its own.
///
/// The same two steps as Run today, and the difference is what happens after:
/// an installer leaves programs behind, and picking one to keep is the panel's
/// own ask (`bottle_programs` + `set_bottle_program`). Saying that here rather
/// than making the two commands identical by accident.
#[tauri::command]
pub async fn windows_file_install(id: u64) -> Result<(), String> {
    into_a_bottle(id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_msi_is_an_installer_and_an_exe_is_not_guessed_at() {
        assert_eq!(kind_of("setup.msi"), FileKind::Installer);
        assert_eq!(kind_of("Setup.MSI"), FileKind::Installer);
        assert_eq!(kind_of("LegacyTool.exe"), FileKind::Portable);
        // Not guessed from a name that merely says "setup": the dialog offers
        // both actions, so a wrong guess would only mislabel what it shows.
        assert_eq!(kind_of("some-setup.exe"), FileKind::Portable);
    }

    #[test]
    fn the_app_name_is_the_file_without_its_extension() {
        assert_eq!(app_name_of("LegacyTool.exe"), "LegacyTool");
        assert_eq!(app_name_of("no-extension"), "no-extension");
        assert_eq!(app_name_of(".hidden"), ".hidden");
    }

    #[test]
    fn a_bottle_id_is_something_a_directory_can_be_called() {
        assert_eq!(bottle_id_for("LegacyTool.exe"), "legacytool");
        assert_eq!(bottle_id_for("Setup (2024)!.exe"), "setup--2024");
        // `...exe` folds to `..`, which names the parent directory. Anything
        // with no letter or digit left falls back instead.
        assert_eq!(bottle_id_for("...exe"), "windows-app");
        assert_eq!(bottle_id_for("--.exe"), "windows-app");
    }

    #[test]
    fn a_prompt_carries_the_name_and_keeps_the_path() {
        let slot = new_slot();
        let ids = Mutex::new(0);
        let r = raise(&slot, "/home/tim/Downloads/LegacyTool.exe", &ids).unwrap();
        assert_eq!(r.file_name, "LegacyTool.exe");
        assert_eq!(r.app_name, "LegacyTool");
        assert_eq!(r.tier, "untested", "nothing here has checked it");
        // The directory is not in what the window is told.
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("Downloads"), "{json}");
        assert_eq!(
            slot.lock().unwrap().as_ref().unwrap().path,
            "/home/tim/Downloads/LegacyTool.exe"
        );
    }

    #[test]
    fn a_second_open_replaces_the_first_and_the_stale_id_acts_on_nothing() {
        let slot = new_slot();
        let ids = Mutex::new(0);
        let first = raise(&slot, "/tmp/a.exe", &ids).unwrap();
        let second = raise(&slot, "/tmp/b.exe", &ids).unwrap();
        assert_ne!(first.id, second.id);
        assert!(take(&slot, first.id).is_none(), "the stale dialog acts on nothing");
        assert_eq!(take(&slot, second.id).unwrap().path, "/tmp/b.exe");
        assert!(take(&slot, second.id).is_none(), "and it is taken once");
    }
}
