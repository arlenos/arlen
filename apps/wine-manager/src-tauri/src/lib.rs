// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The window over the bottles: what exists, what each one may reach, and what
//! could not be read.
//!
//! Everything here is a thin wrapper over `arlen-wine-core`, which is where the
//! rules live and where they are tested. The one thing this layer decides is what
//! a surface is allowed to be told, and it has a rule of its own: a bottle that
//! cannot be read is reported as unreadable, never quietly dropped from the list.
//! A person whose bottle is missing from the window will go looking for it on
//! disk; a person told it will not parse can fix the file.

use std::path::PathBuf;

use arlen_wine_core::bottle::Bottle;
use arlen_wine_core::registry::{bottles_dir, list_bottles};
use serde::Serialize;

/// What reading a bottle's prefix said about its description.
///
/// The description is a claim; the prefix is what Wine and whoever opened the
/// directory have actually written. A window that shows only the claim will keep
/// saying a bottle reaches two folders after `winecfg` has added a third or a
/// re-boot has put `Z:` back.
#[derive(Debug, Serialize)]
pub struct HealthView {
    /// Whether the prefix says the same thing as the description.
    pub agrees: bool,
    /// Granted folders the program cannot see, by letter.
    pub missing: Vec<String>,
    /// Letters in the prefix that no grant asked for.
    pub unexpected: Vec<String>,
    /// Links that leave the prefix with no grant behind them, as paths.
    pub escapes: Vec<String>,
    /// Whether the prefix has been booted at all. A bottle recorded and never run
    /// is not in disagreement with itself.
    pub booted: bool,
}

/// One bottle as the window shows it.
#[derive(Debug, Serialize)]
pub struct BottleView {
    /// The bottle name.
    pub id: String,
    /// Where its prefix lives, shown because a person moving files in and out
    /// needs the path.
    pub prefix: String,
    /// One line per granted directory: the drive letter it became, the host path,
    /// and whether it may be written.
    pub drives: Vec<DriveView>,
    /// What it may reach on the network, as a word the surface can render.
    pub egress: String,
    /// What the prefix itself says, or `None` if it could not be read - which is
    /// a third answer and not the same as agreeing.
    pub health: Option<HealthView>,
}

/// One drive letter as the window shows it.
#[derive(Debug, Serialize)]
pub struct DriveView {
    /// The drive letter.
    pub letter: String,
    /// The host directory behind it.
    pub host: String,
    /// Whether the program may write there.
    pub writable: bool,
}

/// The whole list, including what would not parse.
#[derive(Debug, Serialize)]
pub struct BottleList {
    /// The bottles that read cleanly.
    pub bottles: Vec<BottleView>,
    /// The ones that did not, each with the path and the reason, so the surface
    /// can say which file to look at.
    pub unreadable: Vec<UnreadableBottle>,
}

/// A bottle that is on disk and could not be understood.
#[derive(Debug, Serialize)]
pub struct UnreadableBottle {
    /// The file to look at.
    pub path: String,
    /// What went wrong with it.
    pub reason: String,
}

fn view(b: &Bottle) -> BottleView {
    // The letters come from the same mapping the launcher writes into the prefix,
    // so the window cannot show a letter the program will not see. A grant list
    // that will not map at all (more directories than there are letters) is shown
    // with no drives rather than not shown, since the bottle exists either way.
    let drives = arlen_wine_core::map_drives(&b.grants)
        .unwrap_or_default()
        .into_iter()
        .map(|d| DriveView {
            letter: d.dosdevice_name().to_uppercase(),
            host: d.host.display().to_string(),
            writable: d.access == arlen_wine_core::Access::ReadWrite,
        })
        .collect();
    let health = arlen_wine_core::health::check_bottle(b).ok().map(|h| HealthView {
        agrees: h.agrees(),
        missing: h.missing.iter().map(|c| format!("{c}:")).collect(),
        unexpected: h.unexpected.iter().map(|c| format!("{c}:")).collect(),
        escapes: h.escapes.iter().map(|p| p.display().to_string()).collect(),
        booted: arlen_wine_core::health::is_booted(&b.prefix_root),
    });
    BottleView {
        id: b.id.clone(),
        prefix: b.prefix_root.display().to_string(),
        drives,
        egress: match &b.egress {
            arlen_wine_core::bottle::Egress::None => "none".into(),
            arlen_wine_core::bottle::Egress::Hosts(h) => h.join(", "),
            arlen_wine_core::bottle::Egress::Unrestricted => "unrestricted".into(),
        },
        health,
    }
}

/// Whether this machine can run a Windows program at all.
///
/// The empty list is the reason this exists. "No bottles yet" invites someone to
/// make one, and on an image without Wine there is nothing behind that invitation:
/// `runtime-deps.tsv` records `wineboot` as absent, so a machine can be in a state
/// where the window is correct, the list is honestly empty, and the sentence under
/// it is a promise the system cannot keep. Saying which of the two states you are
/// in costs one `PATH` lookup.
#[derive(Debug, Serialize)]
pub struct Runtime {
    /// Whether the Wine a launch would use is on this machine. The exact binary
    /// `launch_argv` names, so this answer and a launch cannot disagree.
    pub wine: bool,
    /// Where this user's bottles live, when there is a home to keep them in.
    pub bottles_dir: Option<String>,
}

/// Whether the Wine a launch would actually use is on this machine.
///
/// **The path the launcher names, not `PATH`.** This asked whether `wine` was
/// anywhere on `PATH` while `launch_argv` hardcodes `/usr/bin/wine` inside the
/// sandbox, so a machine with Wine at `/usr/local/bin` had a window saying yes
/// and a launch that would refuse. Two places answering the same question two
/// ways is how a control ends up promising what the machine will not do; they
/// now share `launch::WINE`.
///
/// Deliberately not `Command::new(...).spawn()`: asking whether a thing exists
/// by running it is a different question, and running `wine` to find out whether
/// Wine is installed starts a wineserver on a machine that has one.
fn wine_present() -> bool {
    std::path::Path::new(arlen_wine_core::launch::WINE).is_file()
}

/// What this machine can do with a bottle.
#[tauri::command]
fn wine_runtime() -> Runtime {
    Runtime {
        wine: wine_present(),
        // Named so the empty window can say where a bottle would GO. Every other
        // app on this image tells a person that: the calendar names the folder to
        // drop `.ics` files in, mail says a message comes from the file manager.
        // This one explained what a bottle IS and left the reader with no next
        // step, which is the difference between a description and an instruction.
        bottles_dir: dir().ok().map(|d| d.to_string_lossy().into_owned()),
    }
}

/// Where bottles live for this user.
/// Why the bottle list could not be produced, as a word rather than a sentence.
///
/// It was `String(e)` all the way to the screen, so a German reader met an
/// English clause inside a German frame - the same shape as the login screen's
/// English fallback, one layer down. There are exactly two causes and both are
/// nameable here, on the side of the boundary where nobody can translate them.
#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "problem")]
enum BottlesProblem {
    /// Neither `XDG_DATA_HOME` nor `HOME` is set, so there is nowhere bottles
    /// could be kept and nothing to list.
    NoHome,
    /// The bottle directory itself would not be read. `why` survives because it
    /// is what the filesystem said and the only detail anybody has.
    Unreadable { why: String },
}

fn dir() -> Result<PathBuf, String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or("neither XDG_DATA_HOME nor HOME is set, so there is nowhere to keep bottles")?;
    Ok(bottles_dir(&data))
}

/// Every bottle, and every bottle file that would not parse.
#[tauri::command]
fn wine_bottles() -> Result<BottleList, BottlesProblem> {
    let dir = dir().map_err(|_| BottlesProblem::NoHome)?;
    let listing = list_bottles(&dir).map_err(|e| BottlesProblem::Unreadable {
        why: e.to_string(),
    })?;
    Ok(BottleList {
        bottles: listing.bottles.iter().map(view).collect(),
        unreadable: listing
            .unreadable
            .into_iter()
            .map(|(path, reason)| UnreadableBottle {
                path: path.display().to_string(),
                reason,
            })
            .collect(),
    })
}

/// Bring one bottle back to what its description says.
///
/// The window can already say a prefix disagrees; this is the answer to it. Only
/// the doors change - the drive table and the links out - and nothing inside
/// `drive_c` is touched, so a program's own files and settings survive a repair.
/// Returns the reading taken afterwards rather than a success flag, because
/// "what is it now" is the useful answer and "it worked" is not.
#[tauri::command]
fn wine_repair(id: String) -> Result<HealthView, String> {
    let bottles = dir()?;
    let bottle = arlen_wine_core::registry::load_bottle(&bottles, &id).map_err(|e| e.to_string())?;
    let health = arlen_wine_core::health::repair_bottle(&bottle).map_err(|e| e.to_string())?;
    Ok(HealthView {
        agrees: health.agrees(),
        missing: health.missing.iter().map(|c| format!("{c}:")).collect(),
        unexpected: health.unexpected.iter().map(|c| format!("{c}:")).collect(),
        escapes: health.escapes.iter().map(|p| p.display().to_string()).collect(),
        booted: arlen_wine_core::health::is_booted(&bottle.prefix_root),
    })
}

/// Take one drive letter's grant away and make the prefix agree.
///
/// The order is the point. The prefix is written FIRST, because that is what the
/// program actually meets; only once the letter is really gone does the
/// description change to say so. The other order would leave a bottle whose record
/// says a folder is no longer reachable while the symlink is still there - which
/// is the shape a capability browser must never produce, since the whole promise
/// is that pressing revoke revokes.
///
/// Returns the bottle as the window should now show it, so nothing is drawn from
/// an assumption about what the press did.
#[tauri::command]
fn wine_revoke(id: String, letter: String) -> Result<BottleView, String> {
    let bottles = dir()?;
    let bottle = arlen_wine_core::registry::load_bottle(&bottles, &id).map_err(|e| e.to_string())?;
    let letter = letter
        .chars()
        .next()
        .ok_or("no drive letter was named")?;
    let narrowed = arlen_wine_core::health::revoke_grant(&bottle, letter);

    if arlen_wine_core::health::is_booted(&narrowed.prefix_root) {
        let drives = arlen_wine_core::map_drives(&narrowed.grants).map_err(|e| e.to_string())?;
        arlen_wine_core::dosdevices::write_drives(&narrowed.prefix_root, &drives)
            .map_err(|e| e.to_string())?;
    }
    arlen_wine_core::registry::save_bottle(&bottles, &narrowed).map_err(|e| e.to_string())?;
    Ok(view(&narrowed))
}

/// Forget a bottle: its files go to the trash and its record is removed.
///
/// Reversible on purpose. A prefix holds whatever the person installed into the
/// bottle and whatever the program saved there, so it is moved to the trash the
/// file manager can put back rather than unlinked. If the move fails, nothing is
/// removed: a record deleted before its directory moved would leave files nobody
/// owns and a window that cannot offer to clean them up.
///
/// Returns where the files went, so the surface can say it rather than imply it.
#[tauri::command]
fn wine_forget(id: String) -> Result<String, String> {
    let bottles = dir()?;
    let bottle = arlen_wine_core::registry::load_bottle(&bottles, &id).map_err(|e| e.to_string())?;
    let gone = arlen_wine_core::forget::forget_bottle_to_trash(&bottles, &bottle)
        .map_err(|e| e.to_string())?;
    Ok(gone
        .trashed_to
        .map(|p| p.display().to_string())
        .unwrap_or_default())
}

/// Start the window.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![
            wine_bottles,
            wine_runtime,
            wine_repair,
            wine_revoke,
            wine_forget
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_wine_core::bottle::Egress;
    use arlen_wine_core::{Access, PathGrant};

    #[test]
    fn the_runtime_check_asks_about_the_binary_the_launcher_will_run() {
        // This replaced a PATH lookup, and the pair is the point: the window's
        // answer and the launcher's argv have to be about one file, or a machine
        // with Wine somewhere else has a window saying yes and a launch that
        // refuses. Asserting the constant rather than the filesystem, because
        // the answer here depends on whether this particular machine has Wine.
        assert_eq!(arlen_wine_core::launch::WINE, "/usr/bin/wine");
        assert_eq!(
            wine_present(),
            std::path::Path::new(arlen_wine_core::launch::WINE).is_file()
        );
    }

    #[test]
    fn a_bottle_shows_the_letters_its_program_will_see() {
        let b = Bottle {
            id: "notepad".into(),
            prefix_root: PathBuf::from("/data/bottles/notepad/pfx"),
            grants: vec![
                PathGrant { host: PathBuf::from("/home/u/Projects"), access: Access::ReadWrite },
                PathGrant { host: PathBuf::from("/home/u/Docs"), access: Access::ReadOnly },
            ],
            egress: Egress::None,
            plumbing: Default::default(),
        };
        let v = view(&b);
        assert_eq!(v.drives.len(), 2);
        // Sorted by host path, which is what the launcher writes, so the window
        // and the prefix agree on which letter is which.
        assert_eq!(v.drives[0].letter, "D:");
        assert_eq!(v.drives[0].host, "/home/u/Docs");
        assert!(!v.drives[0].writable);
        assert_eq!(v.drives[1].letter, "E:");
        assert!(v.drives[1].writable);
        assert_eq!(v.egress, "none");
    }
}
