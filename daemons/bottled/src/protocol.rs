//! What a caller may ask the bottle daemon, and what it answers.
//!
//! ONE FRAME, ONE QUESTION. The wire is a length-prefixed JSON [`Request`] and a
//! length-prefixed JSON [`Response`], the shape every other broker in this tree
//! uses, so a reader who has seen one has seen this.
//!
//! WHAT IS NOT HERE, and it is most of the Settings panel. That surface renders
//! DLL overrides, winetricks verbs, DXVK, scaling and a window mode; none of those
//! are things a bottle knows about itself - they come from the compat recipe, which
//! `windows-apps-plan.md` lists as its own piece and which does not exist yet. A
//! daemon that answered them would be inventing them, so it answers what a bottle
//! actually carries and the panel keeps saying it has not measured the rest.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::bottle::{Bottle, Egress};
use crate::health::{check_bottle, is_booted};
use crate::launch::{launch_argv, LaunchError};
use crate::map_drives;
use crate::registry::{list_bottles, load_bottle, RegistryError};

/// A question for the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "ask")]
pub enum Request {
    /// Every bottle on this machine, as the panel lists them.
    ListBottles,
    /// One bottle, checked against what is actually on disk.
    Health { id: String },
    /// What this machine can actually run Windows programs with.
    Runtimes,
    /// Forget a bottle: its description is removed and its prefix goes to the
    /// trash.
    ///
    /// TRASH, NOT DELETE, and the core says why in its own code: the prefix holds
    /// whatever the person installed and saved in there, and a button that
    /// destroys that with no way back is not a button worth shipping.
    Forget { id: String },
    /// Where this bottle's C: drive is on the host, so a file manager can open it.
    ///
    /// A PATH, not an open: the daemon does not launch a file manager. It knows
    /// where the prefix is and nothing about the session's opener, and a daemon
    /// that starts a window on the caller's behalf is a wider thing than one that
    /// answers a question.
    Prefix { id: String },
    /// The programs an installer left inside a bottle, for somebody to pick from.
    ///
    /// A LIST, because a Windows installer does not say what it installed and
    /// picking one automatically is a guess. `crate::programs` has the reasoning.
    Programs { id: String },
    /// Record which of a bottle's programs its launch should start.
    ///
    /// The path must be inside the bottle's own prefix, which is checked rather
    /// than trusted: this is what `Launch` runs, so a caller that could name any
    /// host path would have turned a launch into "run what I name, under Wine,
    /// with this bottle's grants".
    SetProgram { id: String, program: String },
    /// Run an installer inside an existing bottle.
    ///
    /// The path is a HOST path, and the daemon copies the file into the prefix
    /// before running it rather than granting the folder it came from - see
    /// `crate::install` for why that trade is the wrong one to make for a one-off.
    ///
    /// Answers when the installer STARTS, not when it finishes: an installer is a
    /// program somebody clicks through, and a caller that waited would hang for as
    /// long as they take to read a licence.
    Install { id: String, installer: String },
    /// Make a bottle: a booted, severed, empty prefix under this id.
    ///
    /// EMPTY IS THE POINT. Nothing is installed by this, and the bottle it leaves
    /// has no program to run - `Launch` refuses it as `nothing-to-run` until an
    /// installer has been through. Making the container and putting something in
    /// it are two asks because they fail differently and a caller needs to know
    /// which happened.
    ///
    /// Not gated the way `Forget` is: this creates rather than destroys, and a
    /// bottle with no grants and no network is the least a caller can ask for.
    Create { id: String },
    /// Remove the regenerable caches inside a bottle's prefix.
    ///
    /// Not gated the way [`Request::Forget`] is, and the difference is what is at
    /// stake: forgetting throws away what somebody installed, while everything a
    /// sweep removes is rebuilt the next time the program runs. `crate::caches`
    /// documents what counts as regenerable and why the walk never follows a link.
    ClearCaches { id: String },
    /// Start the Windows program this bottle exists to run.
    ///
    /// The caller names the bottle, not the program: what runs is what the bottle
    /// records, so a caller cannot use this to run something else inside somebody
    /// else's confinement.
    Launch { id: String },
}

/// One bottle as a caller sees it.
///
/// DERIVED, never stored: `network` and `home_folder` are read off the bottle's
/// egress and grants at answer time, so a panel cannot show a permission the
/// bottle stopped having.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BottleView {
    pub id: String,
    /// Whether this bottle may reach the network at all.
    pub network: bool,
    /// Whether one of its granted drives is the person's home.
    pub home_folder: bool,
    /// The drives it was granted, in letter order. Derived through the same
    /// mapping the launcher runs, so a surface and the bottle cannot disagree
    /// about which letter is which.
    pub drives: Vec<DriveView>,
    /// Whether a program has been picked for this bottle yet.
    ///
    /// Carried because the panel has something to ASK when it is false: an
    /// installer has run and nobody has said which of what it left is the app, so
    /// a launch would refuse as `nothing-to-run`. Without this the surface cannot
    /// tell an empty new bottle from one that is ready, and the person meets the
    /// refusal instead of the question.
    pub has_program: bool,
}

/// One program found inside a bottle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramView {
    /// The host path, which is what a launch would run and what `SetProgram`
    /// takes back.
    pub path: String,
    /// The file name, which is what a person recognises in a list.
    pub name: String,
}

/// One granted drive: the letter a Windows program sees, and the host folder it
/// really is. Both, because a letter on its own says a drive exists without
/// saying what it reaches, which is the half that matters for confinement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveView {
    pub letter: char,
    pub path: String,
}

/// What the daemon says back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "answer")]
pub enum Response {
    Bottles {
        bottles: Vec<BottleView>,
        /// The bottles that are there and did not read.
        ///
        /// CARRIED, not dropped. `Listing` keeps these apart from the ones that
        /// read cleanly for the reason its own doc gives - so the surface can say
        /// so - and the first cut of this answer threw them away, which turns "one
        /// of your bottles is broken" into "you have no bottles". Measured against
        /// a bottle.toml that did not parse: the list came back empty and cheerful
        /// while a Health ask on the same id said it could not be read.
        unreadable: Vec<String>,
    },
    /// What the prefix on disk says, against what the bottle says it is.
    Health {
        /// Whether the prefix has been booted at all.
        booted: bool,
        /// Whether the two descriptions agree.
        agrees: bool,
        /// Letters the bottle expects and the prefix does not have.
        missing: Vec<char>,
        /// Letters in the prefix no grant asked for.
        unexpected: Vec<char>,
        /// Links that leave the prefix without a grant behind them.
        escapes: usize,
    },
    /// The program was started, and this is the process it became.
    ///
    /// STARTED, not finished: the daemon does not wait for it. That is the whole
    /// reason this is a daemon - the program outlives the window that asked for
    /// it. It does not outlive the SESSION: the shared confiner passes
    /// `--die-with-parent`, so when the daemon goes, so does the program, which is
    /// the lifetime a desktop app should have.
    Launched { pid: u32 },
    /// The compatibility runtimes on this machine.
    ///
    /// MEASURED, never listed from a catalogue of things that might exist. The
    /// panel used to show "Wine 9.0 installed, Proton 9.0 installed, DXVK 2.4
    /// installed" as an opening value, which stated the contents of a disk nobody
    /// had read. `wine` is `None` when there is none, which is a fact a person
    /// needs before they wonder why nothing starts.
    Runtimes { wine: Option<String> },
    /// The bottle is gone, and this is where its prefix went.
    ///
    /// `trashed_to` is `None` when there was no prefix on disk to move - the
    /// description was removed and nothing else existed. Said rather than implied,
    /// so a window can tell somebody where their files are.
    Forgotten { trashed_to: Option<String> },
    /// The host path of the bottle's C: drive.
    ///
    /// `drive_c` rather than the prefix root: that is the folder the person's
    /// program installed itself into, and the root beside it holds the registry
    /// and the drive table, which are ours rather than theirs.
    Prefix { path: String },
    /// The programs found inside a bottle, sorted by path.
    ///
    /// An empty list is a real answer: an installer that was cancelled leaves
    /// nothing behind, and saying so beats offering an invented entry.
    ///
    /// `truncated` says the list was CUT to fit the wire, which the caller has to
    /// be able to tell from "that is all of them". The frame cap is 64 KiB and a
    /// path can be 4096 bytes, so a count alone cannot guarantee a fit - fifteen
    /// deep paths already exceed it. An over-cap answer is not a truncated answer:
    /// `write_frame` refuses it and the connection is dropped, so the caller sees
    /// a transport error and renders "nothing found" for a bottle full of
    /// programs. Measured while writing this, not in the field.
    Programs {
        programs: Vec<ProgramView>,
        truncated: bool,
    },
    /// The program was recorded, and this is what a launch will now start.
    ProgramSet { program: String },
    /// The bottle was made, and this is its id.
    ///
    /// The id is echoed rather than assumed: a caller that let the daemon settle
    /// the name would otherwise have to guess at it.
    Created { id: String },
    /// The sweep is done, and this is what it actually removed.
    ///
    /// MEASURED, not promised: a bottle with no caches answers zero, which is a
    /// true sentence a surface may show instead of implying it reclaimed
    /// something.
    Cleared { bytes: u64, files: usize },
    /// The ask could not be answered. A token, not prose: the window writes the
    /// sentence, in the reader's language.
    Refused { problem: Problem },
}

/// Why an ask was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Problem {
    /// No bottle by that id.
    NoSuchBottle,
    /// The id was not a name a bottle may have.
    BadId,
    /// The bottles directory could not be read.
    Unreadable,
    /// The bottle is there and has no program recorded, so there is nothing to
    /// start. Distinct from every failure below: nothing went wrong.
    NothingToRun,
    /// There is no Wine on this machine, so no Windows program can run at all.
    NoWine,
    /// The bottle's prefix is not on disk, so there is nothing to run inside.
    ///
    /// Refused for the reason `launch.rs` gives for the two above it: without the
    /// check the argv is built, bwrap starts, and the failure surfaces from inside
    /// the sandbox as `Can't find source path` - which names neither the prefix
    /// nor the fact that this bottle was never made. Measured, by launching one:
    /// the daemon answered `launched` and a pid for a program that was already
    /// dead.
    PrefixMissing,
    /// The drive table promises reach the confinement does not give. Refused
    /// rather than started: the program would see a drive it cannot open.
    DrivesUnmet,
    /// The confinement could not be started.
    CouldNotStart,
    /// There is already a bottle by that name. Refused rather than merged: a
    /// bottle is a prefix with software in it, and overwriting one because the
    /// name matched would throw that away.
    BottleExists,
    /// The bottle could not be made. Wine would not boot the prefix, the prefix
    /// booted and still reached out of itself, or the disk said no - one token,
    /// because none of the three is something the person can act on differently.
    CouldNotCreate,
    /// The program named is not a file inside this bottle's prefix. Refused for
    /// the reason `SetProgram` gives: what a bottle runs must be what is in it.
    NotInThisBottle,
    /// The installer named is not a file this machine has, or is not a file at
    /// all. Said rather than attempted: a copy of a directory fails deep inside an
    /// operation the person did not ask about.
    NoInstaller,
    /// The caller may not do this. Forgetting a bottle throws away what somebody
    /// installed, so it is not something any process that can reach the socket
    /// gets to ask for.
    NotAllowed,
    /// The bottle could not be forgotten - the trash refused it, or its files
    /// would not move. Nothing was half-removed: the description goes last.
    CouldNotForget,
}

/// A bottle as a caller sees it.
pub fn view(bottle: &Bottle) -> BottleView {
    BottleView {
        id: bottle.id.clone(),
        has_program: !bottle.program.is_empty(),
        network: !matches!(bottle.egress, Egress::None),
        home_folder: std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .is_some_and(|home| bottle.grants.iter().any(|g| g.host == home)),
        // An unmappable grant set yields no letters rather than a guess: the same
        // refusal the launcher makes, so a panel never shows a drive that would
        // not be there when the program starts.
        drives: map_drives(&bottle.grants)
            .map(|drives| {
                drives
                    .iter()
                    .map(|d| DriveView {
                        letter: d.letter,
                        path: d.host.to_string_lossy().into_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Start a bottle's own program, and answer with the process it became.
///
/// The spawn is injected so the refusals can be tested without putting a Windows
/// program on a build machine: everything up to the spawn is this crate's, and
/// what the spawn does with the argv is the caller's.
pub fn launch(
    bottles_dir: &Path,
    id: &str,
    usr: &Path,
    runtime_dir: &Path,
    display: Option<&str>,
    exists: impl Fn(&Path) -> bool,
    spawn: impl FnOnce(&[String]) -> std::io::Result<u32>,
) -> Response {
    let bottle = match load_bottle(bottles_dir, id) {
        Ok(b) => b,
        Err(RegistryError::BadId(_)) => {
            return Response::Refused {
                problem: Problem::BadId,
            }
        }
        Err(RegistryError::NoSuchBottle(_)) => {
            return Response::Refused {
                problem: Problem::NoSuchBottle,
            }
        }
        Err(_) => {
            return Response::Refused {
                problem: Problem::Unreadable,
            }
        }
    };
    // Asked before the argv is built, so "you have not installed anything in this
    // bottle yet" does not arrive as "no program was named", which reads like a
    // caller mistake.
    if bottle.program.is_empty() {
        return Response::Refused {
            problem: Problem::NothingToRun,
        };
    }
    if !exists(&bottle.prefix_root) {
        return Response::Refused {
            problem: Problem::PrefixMissing,
        };
    }
    let argv = match launch_argv(&bottle, usr, runtime_dir, display, &bottle.program, exists) {
        Ok(v) => v,
        Err(LaunchError::NoRuntime(_)) => {
            return Response::Refused {
                problem: Problem::NoWine,
            }
        }
        Err(LaunchError::UnmetDrives(_)) => {
            return Response::Refused {
                problem: Problem::DrivesUnmet,
            }
        }
        Err(_) => {
            return Response::Refused {
                problem: Problem::CouldNotStart,
            }
        }
    };
    match spawn(&argv) {
        Ok(pid) => Response::Launched { pid },
        Err(_) => Response::Refused {
            problem: Problem::CouldNotStart,
        },
    }
}

/// Forget a bottle, and answer with where its prefix went.
///
/// The trash is injected for the same reason `forget_bottle` injects it: a machine
/// with no home trash should decide what to do rather than have this decide for
/// it, and the sequence is testable without one.
pub fn forget(
    bottles_dir: &Path,
    id: &str,
    trash: impl Fn(&Path) -> Result<std::path::PathBuf, String>,
) -> Response {
    let bottle = match load_bottle(bottles_dir, id) {
        Ok(b) => b,
        Err(RegistryError::BadId(_)) => {
            return Response::Refused {
                problem: Problem::BadId,
            }
        }
        Err(RegistryError::NoSuchBottle(_)) => {
            return Response::Refused {
                problem: Problem::NoSuchBottle,
            }
        }
        Err(_) => {
            return Response::Refused {
                problem: Problem::Unreadable,
            }
        }
    };
    match crate::forget::forget_bottle(bottles_dir, &bottle, trash) {
        Ok(gone) => Response::Forgotten {
            trashed_to: gone.trashed_to.map(|p| p.to_string_lossy().into_owned()),
        },
        Err(_) => Response::Refused {
            problem: Problem::CouldNotForget,
        },
    }
}

/// Bring an installer into a bottle and start it.
///
/// Reuses the launch assembly with the copied-in file as the program, so an
/// installer runs under exactly the confinement the bottle's own software will:
/// there is no wider "install mode", and a bottle that may not reach the network
/// may not reach it while installing either.
///
/// Answers `Launched` with the installer's pid. What it installed is a separate
/// question - the prefix has to be looked at afterwards - and a separate ask.
#[allow(clippy::too_many_arguments)]
pub fn install(
    bottles_dir: &Path,
    id: &str,
    installer: &str,
    usr: &Path,
    runtime_dir: &Path,
    display: Option<&str>,
    exists: impl Fn(&Path) -> bool,
    spawn: impl FnOnce(&[String]) -> std::io::Result<u32>,
) -> Response {
    let bottle = match load_bottle(bottles_dir, id) {
        Ok(b) => b,
        Err(RegistryError::BadId(_)) => {
            return Response::Refused {
                problem: Problem::BadId,
            }
        }
        Err(RegistryError::NoSuchBottle(_)) => {
            return Response::Refused {
                problem: Problem::NoSuchBottle,
            }
        }
        Err(_) => {
            return Response::Refused {
                problem: Problem::Unreadable,
            }
        }
    };
    if !exists(&bottle.prefix_root) {
        return Response::Refused {
            problem: Problem::PrefixMissing,
        };
    }
    let landed = match crate::install::bring_installer_in(
        &bottle.prefix_root,
        std::path::Path::new(installer),
    ) {
        Ok(p) => p,
        // The copy and the name are one answer to the caller: both mean the file
        // they named is not one that can be brought in.
        Err(crate::install::InstallError::NotAFile(_))
        | Err(crate::install::InstallError::BadName(_)) => {
            return Response::Refused {
                problem: Problem::NoInstaller,
            }
        }
        Err(crate::install::InstallError::Io(_)) => {
            return Response::Refused {
                problem: Problem::CouldNotStart,
            }
        }
    };

    let program = vec![landed.to_string_lossy().into_owned()];
    let argv = match launch_argv(&bottle, usr, runtime_dir, display, &program, exists) {
        Ok(v) => v,
        Err(LaunchError::NoRuntime(_)) => {
            return Response::Refused {
                problem: Problem::NoWine,
            }
        }
        Err(LaunchError::UnmetDrives(_)) => {
            return Response::Refused {
                problem: Problem::DrivesUnmet,
            }
        }
        Err(_) => {
            return Response::Refused {
                problem: Problem::CouldNotStart,
            }
        }
    };
    match spawn(&argv) {
        Ok(pid) => Response::Launched { pid },
        Err(_) => Response::Refused {
            problem: Problem::CouldNotStart,
        },
    }
}

/// Make a bottle, booting its prefix through the runner the caller supplies.
///
/// The runner is injected for the same reason `launch`'s spawn is: the sequence
/// can then be exercised on a machine with no Wine, and the caller decides whether
/// the boot is confined. The server's is - it runs [`crate::create::boot_argv`]
/// under bwrap.
///
/// A failed boot leaves nothing behind. `create_bottle` writes the description
/// last, so a prefix that did not finish is a directory with no `bottle.toml`,
/// which the registry deliberately neither lists nor reports.
pub fn create(
    bottles_dir: &Path,
    id: &str,
    usr: &Path,
    runtime_dir: &Path,
    exists: impl Fn(&Path) -> bool + Copy,
    run: impl Fn(&[String]) -> Result<(), String>,
) -> Response {
    let new = crate::create::NewBottle {
        id: id.to_string(),
        // Nothing of the person's, and no network. A bottle is granted its reach
        // deliberately and afterwards; one that arrives with reach it was never
        // asked for is the shape this whole daemon exists to avoid.
        grants: Vec::new(),
        egress: crate::bottle::Egress::None,
        plumbing: crate::plumbing::Plumbing::default(),
    };
    let boot = |prefix: &Path| -> Result<(), String> {
        let argv = crate::create::boot_argv(prefix, usr, runtime_dir, exists)
            .map_err(|e| e.to_string())?;
        run(&argv)
    };
    match crate::create::create_bottle(bottles_dir, &new, boot) {
        Ok(bottle) => Response::Created { id: bottle.id },
        Err(crate::create::CreateError::AlreadyExists(_)) => Response::Refused {
            problem: Problem::BottleExists,
        },
        Err(crate::create::CreateError::Registry(RegistryError::BadId(_))) => Response::Refused {
            problem: Problem::BadId,
        },
        // Boot, StillEscapes, Drives, Io and the rest of the registry errors are
        // one token: each is the machine failing rather than the person asking for
        // something impossible, and a surface has one sentence for that.
        Err(_) => Response::Refused {
            problem: Problem::CouldNotCreate,
        },
    }
}

/// How many bytes of program list may travel, leaving the rest of the frame to the
/// JSON around it. Well under the 64 KiB cap on purpose: the margin is not tuned,
/// it is generous, because the cost of getting it wrong is a dropped connection
/// that reads as an empty bottle.
const PROGRAM_LIST_BUDGET: usize = 32 * 1024;

/// Take as many programs as fit the budget, and say whether any were left.
///
/// Bounded by SIZE rather than by count, because the thing that has to fit is
/// bytes: a path may be 4096 of them, so any count small enough to be safe would
/// be too small to be useful for the ordinary case of short ones.
fn fit_programs(all: Vec<ProgramView>) -> (Vec<ProgramView>, bool) {
    let mut kept = Vec::new();
    let mut used = 0;
    let total = all.len();
    for p in all {
        // The two strings plus the JSON around them, near enough: the budget's
        // margin covers the difference between this and the exact encoding.
        let cost = p.path.len() + p.name.len() + 32;
        if used + cost > PROGRAM_LIST_BUDGET {
            break;
        }
        used += cost;
        kept.push(p);
    }
    let truncated = kept.len() < total;
    (kept, truncated)
}

/// Answer one request against a bottles directory, or say it is not this
/// function's to answer.
///
/// Pure over the filesystem it is handed, so the reading vocabulary is testable
/// without a socket, a peer or a running Wine. `None` is [`Request::Launch`],
/// which needs the host it will run on - where `/usr` is, the runtime dir, the
/// display - and is answered by [`launch`] instead. Returning `None` rather than a
/// refusal keeps that boundary honest: nothing was attempted, so no refusal would
/// be true.
pub fn handle_request(bottles_dir: &Path, request: &Request) -> Option<Response> {
    Some(match request {
        Request::ListBottles => match list_bottles(bottles_dir) {
            Ok(listing) => Response::Bottles {
                bottles: listing.bottles.iter().map(view).collect(),
                unreadable: listing
                    .unreadable
                    .iter()
                    .map(|(path, _why)| {
                        // The directory name is the bottle id; the reason stays
                        // here rather than travelling, because it is a parser's
                        // words and the window writes the sentence.
                        path.parent()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.to_string_lossy().into_owned())
                    })
                    .collect(),
            },
            Err(_) => Response::Refused {
                problem: Problem::Unreadable,
            },
        },
        // Neither is answered here: a launch needs the host it will run on, and a
        // forget needs a trash and a caller allowed to ask. Both are the server's.
        // Neither is answered here: a launch needs the host it will run on, a
        // forget needs a trash and a caller allowed to ask, and reading a runtime
        // version means running the thing. All three are the server's.
        Request::Launch { .. }
        | Request::Forget { .. }
        | Request::Runtimes
        // A create needs the host too: booting a prefix means running Wine on
        // this machine, which is exactly what `handle_request` is pure of. So does
        // an install, which runs one.
        | Request::Create { .. }
        | Request::Install { .. } => return None,
        Request::Programs { id } => match load_bottle(bottles_dir, id) {
            Ok(bottle) => {
                let all: Vec<ProgramView> = crate::programs::candidates(&bottle.prefix_root)
                    .into_iter()
                    .map(|c| ProgramView {
                        path: c.path.to_string_lossy().into_owned(),
                        name: c.name,
                    })
                    .collect();
                let (programs, truncated) = fit_programs(all);
                Response::Programs {
                    programs,
                    truncated,
                }
            }
            Err(RegistryError::BadId(_)) => Response::Refused {
                problem: Problem::BadId,
            },
            Err(RegistryError::NoSuchBottle(_)) => Response::Refused {
                problem: Problem::NoSuchBottle,
            },
            Err(_) => Response::Refused {
                problem: Problem::Unreadable,
            },
        },
        Request::SetProgram { id, program } => match load_bottle(bottles_dir, id) {
            Ok(mut bottle) => {
                if !crate::programs::is_inside_prefix(
                    &bottle.prefix_root,
                    std::path::Path::new(program),
                ) {
                    Response::Refused {
                        problem: Problem::NotInThisBottle,
                    }
                } else {
                    // One element: the argv a launch runs is the program and
                    // nothing else. Arguments are a compat-recipe field, and this
                    // is not the place to invent one.
                    bottle.program = vec![program.clone()];
                    match crate::registry::save_bottle(bottles_dir, &bottle) {
                        Ok(_) => Response::ProgramSet {
                            program: program.clone(),
                        },
                        Err(_) => Response::Refused {
                            problem: Problem::Unreadable,
                        },
                    }
                }
            }
            Err(RegistryError::BadId(_)) => Response::Refused {
                problem: Problem::BadId,
            },
            Err(RegistryError::NoSuchBottle(_)) => Response::Refused {
                problem: Problem::NoSuchBottle,
            },
            Err(_) => Response::Refused {
                problem: Problem::Unreadable,
            },
        },
        Request::ClearCaches { id } => match load_bottle(bottles_dir, id) {
            Ok(bottle) => {
                let cleared = crate::caches::clear_caches(&bottle.prefix_root);
                Response::Cleared {
                    bytes: cleared.bytes,
                    files: cleared.files,
                }
            }
            Err(RegistryError::BadId(_)) => Response::Refused {
                problem: Problem::BadId,
            },
            Err(RegistryError::NoSuchBottle(_)) => Response::Refused {
                problem: Problem::NoSuchBottle,
            },
            Err(_) => Response::Refused {
                problem: Problem::Unreadable,
            },
        },
        Request::Prefix { id } => match load_bottle(bottles_dir, id) {
            Ok(bottle) => {
                let drive_c = bottle.prefix_root.join("drive_c");
                if drive_c.is_dir() {
                    Response::Prefix {
                        path: drive_c.to_string_lossy().into_owned(),
                    }
                } else {
                    // A bottle that has never booted has no C: drive, and handing
                    // back a path that is not there would open a file manager on
                    // nothing. `PrefixMissing` is the same refusal a launch makes
                    // for the same absence, so the window has one sentence for it.
                    Response::Refused {
                        problem: Problem::PrefixMissing,
                    }
                }
            }
            Err(RegistryError::BadId(_)) => Response::Refused {
                problem: Problem::BadId,
            },
            Err(RegistryError::NoSuchBottle(_)) => Response::Refused {
                problem: Problem::NoSuchBottle,
            },
            Err(_) => Response::Refused {
                problem: Problem::Unreadable,
            },
        },
        Request::Health { id } => match load_bottle(bottles_dir, id) {
            Ok(bottle) => match check_bottle(&bottle) {
                Ok(health) => Response::Health {
                    booted: is_booted(&bottle.prefix_root),
                    agrees: health.agrees(),
                    missing: health.missing,
                    unexpected: health.unexpected,
                    escapes: health.escapes.len(),
                },
                Err(_) => Response::Refused {
                    problem: Problem::Unreadable,
                },
            },
            // The three are kept apart on purpose: a name that could never be a
            // bottle, a name that simply is not one here, and a bottle that is
            // there and unreadable are different answers, and only the last is a
            // fault the person can do something about.
            Err(RegistryError::BadId(_)) => Response::Refused {
                problem: Problem::BadId,
            },
            Err(RegistryError::NoSuchBottle(_)) => Response::Refused {
                problem: Problem::NoSuchBottle,
            },
            Err(_) => Response::Refused {
                problem: Problem::Unreadable,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_machine_lists_no_bottles() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            handle_request(dir.path(), &Request::ListBottles),
            Some(Response::Bottles {
                bottles: vec![],
                unreadable: vec![]
            }),
            "a machine with no bottles has none, which is not a failure to read them"
        );
    }

    #[test]
    fn a_bottle_that_will_not_read_is_named_rather_than_dropped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("broken")).unwrap();
        std::fs::write(dir.path().join("broken/bottle.toml"), "id = [not toml").unwrap();

        let Some(Response::Bottles {
            bottles,
            unreadable,
        }) = handle_request(dir.path(), &Request::ListBottles)
        else {
            panic!("a list ask is answered with a list");
        };
        assert!(bottles.is_empty());
        assert_eq!(
            unreadable,
            vec!["broken".to_string()],
            "silently, this reads as a machine with no bottles at all"
        );
    }

    #[test]
    fn the_three_ways_a_health_ask_can_miss_stay_apart() {
        let dir = tempfile::tempdir().unwrap();

        // A name no bottle may have, and a name that simply is not here, are
        // different answers - the first is the caller's mistake, the second is an
        // ordinary empty machine.
        assert_eq!(
            handle_request(
                dir.path(),
                &Request::Health {
                    id: "../etc".into()
                }
            ),
            Some(Response::Refused {
                problem: Problem::BadId
            })
        );
        assert_eq!(
            handle_request(
                dir.path(),
                &Request::Health {
                    id: "not-here".into()
                }
            ),
            Some(Response::Refused {
                problem: Problem::NoSuchBottle
            })
        );
    }

    #[test]
    fn a_prefix_ask_answers_the_c_drive_and_refuses_one_that_was_never_booted() {
        use crate::registry::save_bottle;

        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("game/pfx");
        let bottle = Bottle {
            id: "game".into(),
            prefix_root: prefix.clone(),
            grants: vec![],
            egress: Egress::None,
            plumbing: Default::default(),
            program: vec![],
        };
        save_bottle(dir.path(), &bottle).unwrap();

        // Made but never booted: there is no C: drive to open, and the answer says
        // so rather than handing back a path to nothing.
        assert_eq!(
            handle_request(dir.path(), &Request::Prefix { id: "game".into() }),
            Some(Response::Refused {
                problem: Problem::PrefixMissing
            }),
            "a prefix with no drive_c is not a folder anybody can be shown"
        );

        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        assert_eq!(
            handle_request(dir.path(), &Request::Prefix { id: "game".into() }),
            Some(Response::Prefix {
                path: prefix.join("drive_c").to_string_lossy().into_owned()
            })
        );

        // The same three answers the other reading asks give, so a window has one
        // vocabulary for "that is not a name" and "that is not here".
        assert_eq!(
            handle_request(dir.path(), &Request::Prefix { id: "../etc".into() }),
            Some(Response::Refused {
                problem: Problem::BadId
            })
        );
        assert_eq!(
            handle_request(dir.path(), &Request::Prefix { id: "absent".into() }),
            Some(Response::Refused {
                problem: Problem::NoSuchBottle
            })
        );
    }

    #[test]
    fn a_create_makes_an_empty_bottle_and_refuses_a_second_one_by_that_name() {
        let dir = tempfile::tempdir().unwrap();
        // The boot is the injected step: it makes the directories Wine would, so
        // the severing and drive-table passes after it have something to read.
        let booted = |prefix: &[String]| -> Result<(), String> {
            let _ = prefix;
            Ok(())
        };
        let make = |dir: &Path| {
            create(
                dir,
                "game",
                Path::new("/usr"),
                Path::new("/run/user/1000"),
                |_| true,
                |argv: &[String]| {
                    // The confiner emits an environment as `--setenv NAME VALUE`,
                    // three elements, so the prefix is the one after the name. A
                    // real wineboot creates that directory; this stands in for it.
                    let prefix = argv
                        .iter()
                        .position(|a| a == "WINEPREFIX")
                        .and_then(|i| argv.get(i + 1))
                        .map(std::path::PathBuf::from)
                        .expect("a boot names its prefix");
                    std::fs::create_dir_all(prefix.join("dosdevices")).unwrap();
                    booted(argv)
                },
            )
        };

        assert_eq!(
            make(dir.path()),
            Response::Created { id: "game".into() },
            "a bottle is made and answers with the name it was given"
        );
        // Empty, which is the whole contract: nothing is installed and a launch
        // says so rather than starting something.
        assert_eq!(
            handle_request(dir.path(), &Request::ListBottles),
            Some(Response::Bottles {
                bottles: vec![BottleView {
                    id: "game".into(),
                    network: false,
                    home_folder: false,
                    drives: vec![],
                    has_program: false,
                }],
                unreadable: vec![]
            })
        );
        assert_eq!(
            make(dir.path()),
            Response::Refused {
                problem: Problem::BottleExists
            },
            "a second bottle by that name would overwrite what is installed in the first"
        );
    }

    #[test]
    fn a_create_on_a_machine_without_wine_says_so_rather_than_leaving_a_half_bottle() {
        let dir = tempfile::tempdir().unwrap();
        let answer = create(
            dir.path(),
            "game",
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            |_| false,
            |_| Ok(()),
        );
        assert_eq!(
            answer,
            Response::Refused {
                problem: Problem::CouldNotCreate
            }
        );
        assert_eq!(
            handle_request(dir.path(), &Request::ListBottles),
            Some(Response::Bottles {
                bottles: vec![],
                unreadable: vec![]
            }),
            "a create that failed leaves nothing to list and nothing to report broken"
        );
    }

    #[test]
    fn an_install_copies_the_file_in_and_runs_it_from_inside_the_bottle() {
        use crate::registry::save_bottle;

        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("game/pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        save_bottle(
            dir.path(),
            &Bottle {
                id: "game".into(),
                prefix_root: prefix.clone(),
                grants: vec![],
                egress: Egress::None,
                plumbing: Default::default(),
                program: vec![],
            },
        )
        .unwrap();

        let downloads = dir.path().join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let installer = downloads.join("setup.exe");
        std::fs::write(&installer, b"MZ").unwrap();

        let seen = std::cell::RefCell::new(Vec::new());
        let answer = install(
            dir.path(),
            "game",
            installer.to_str().unwrap(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            |_| true,
            |argv| {
                *seen.borrow_mut() = argv.to_vec();
                Ok(4242)
            },
        );
        assert_eq!(answer, Response::Launched { pid: 4242 });

        let argv = seen.into_inner();
        let program = argv.last().expect("something is run");
        assert_eq!(
            program,
            &prefix
                .join(crate::install::INSTALLER_DIR)
                .join("setup.exe")
                .to_string_lossy()
                .into_owned(),
            "what runs is the copy inside the prefix, not the file in Downloads"
        );
        assert!(
            !argv.iter().any(|a| a == downloads.to_str().unwrap()),
            "the folder the installer came from is never bound in - that grant would \
             outlive the install and carry everything else in the folder with it"
        );
    }

    #[test]
    fn an_install_from_something_that_is_not_a_file_is_refused_before_anything_runs() {
        use crate::registry::save_bottle;

        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("game/pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        save_bottle(
            dir.path(),
            &Bottle {
                id: "game".into(),
                prefix_root: prefix,
                grants: vec![],
                egress: Egress::None,
                plumbing: Default::default(),
                program: vec![],
            },
        )
        .unwrap();

        assert_eq!(
            install(
                dir.path(),
                "game",
                "/nonexistent/setup.exe",
                Path::new("/usr"),
                Path::new("/run/user/1000"),
                None,
                |_| true,
                |_| panic!("nothing runs when there is nothing to install"),
            ),
            Response::Refused {
                problem: Problem::NoInstaller
            }
        );
    }

    #[test]
    fn picking_a_program_makes_a_bottle_launchable_and_only_from_inside_it() {
        use crate::registry::{load_bottle, save_bottle};

        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("game/pfx");
        let app = prefix.join("drive_c/Program Files/Game/game.exe");
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::write(&app, b"MZ").unwrap();
        save_bottle(
            dir.path(),
            &Bottle {
                id: "game".into(),
                prefix_root: prefix.clone(),
                grants: vec![],
                egress: Egress::None,
                plumbing: Default::default(),
                program: vec![],
            },
        )
        .unwrap();

        // What the installer left, as the panel would list it.
        let Some(Response::Programs {
            programs,
            truncated,
        }) = handle_request(dir.path(), &Request::Programs { id: "game".into() })
        else {
            panic!("a programs ask is answered with programs");
        };
        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].name, "game.exe");
        assert!(!truncated, "one program is all of them");

        // A path outside the bottle is refused, and the bottle is left alone.
        let elsewhere = dir.path().join("outside.exe");
        std::fs::write(&elsewhere, b"MZ").unwrap();
        assert_eq!(
            handle_request(
                dir.path(),
                &Request::SetProgram {
                    id: "game".into(),
                    program: elsewhere.to_string_lossy().into_owned(),
                }
            ),
            Some(Response::Refused {
                problem: Problem::NotInThisBottle
            })
        );
        assert!(
            load_bottle(dir.path(), "game").unwrap().program.is_empty(),
            "a refused pick records nothing"
        );

        // The real one is recorded, and the bottle stops being nothing-to-run.
        assert_eq!(
            handle_request(
                dir.path(),
                &Request::SetProgram {
                    id: "game".into(),
                    program: programs[0].path.clone(),
                }
            ),
            Some(Response::ProgramSet {
                program: programs[0].path.clone()
            })
        );
        assert_eq!(
            load_bottle(dir.path(), "game").unwrap().program,
            vec![programs[0].path.clone()]
        );
    }

    #[test]
    fn a_list_too_long_for_the_wire_is_cut_and_says_so() {
        // Long paths, because that is what makes a count-based cap unsafe: the
        // frame is 64 KiB and a single path may be 4096 bytes.
        let deep = "d".repeat(300);
        let all: Vec<ProgramView> = (0..400)
            .map(|i| ProgramView {
                path: format!("/pfx/drive_c/{deep}/app{i}.exe"),
                name: format!("app{i}.exe"),
            })
            .collect();

        let (kept, truncated) = fit_programs(all);
        assert!(truncated, "400 long paths do not fit and the caller must be told");
        assert!(!kept.is_empty(), "what does fit is still answered");

        let framed = serde_json::to_vec(&Response::Programs {
            programs: kept,
            truncated,
        })
        .unwrap();
        assert!(
            framed.len() < crate::server::MAX_FRAME,
            "the whole point: an over-cap answer is refused by write_frame and the \
             connection is dropped, which the panel renders as an empty bottle"
        );

        // A list that fits is not reported as cut.
        let short = vec![ProgramView {
            path: "/pfx/drive_c/a.exe".into(),
            name: "a.exe".into(),
        }];
        let (kept, truncated) = fit_programs(short);
        assert_eq!(kept.len(), 1);
        assert!(!truncated);
    }

    #[test]
    fn a_clear_ask_answers_what_it_removed() {
        use crate::registry::save_bottle;

        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("game/pfx");
        std::fs::create_dir_all(prefix.join("drive_c/windows/temp")).unwrap();
        std::fs::write(prefix.join("drive_c/windows/temp/setup.dat"), vec![0u8; 20]).unwrap();
        save_bottle(
            dir.path(),
            &Bottle {
                id: "game".into(),
                prefix_root: prefix.clone(),
                grants: vec![],
                egress: Egress::None,
                plumbing: Default::default(),
                program: vec![],
            },
        )
        .unwrap();

        assert_eq!(
            handle_request(dir.path(), &Request::ClearCaches { id: "game".into() }),
            Some(Response::Cleared {
                bytes: 20,
                files: 1
            })
        );
        // Twice, because a surface that offers the button again should get a true
        // zero rather than a repeat of the first number.
        assert_eq!(
            handle_request(dir.path(), &Request::ClearCaches { id: "game".into() }),
            Some(Response::Cleared {
                bytes: 0,
                files: 0
            })
        );
        assert_eq!(
            handle_request(dir.path(), &Request::ClearCaches { id: "absent".into() }),
            Some(Response::Refused {
                problem: Problem::NoSuchBottle
            })
        );
    }

    #[test]
    fn a_launch_says_which_thing_stopped_it() {
        let dir = tempfile::tempdir().unwrap();
        let never = |_: &[String]| -> std::io::Result<u32> {
            panic!("nothing may be spawned once the launch is refused")
        };

        // A bottle with nothing installed in it yet. "You have not put anything
        // in here" is not a failure, and must not arrive as one.
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        std::fs::write(
            dir.path().join("empty/bottle.toml"),
            "id = \"empty\"\nprefix_root = \"/nowhere/pfx\"\negress = \"none\"\ngrants = []\n",
        )
        .unwrap();
        assert_eq!(
            launch(
                dir.path(),
                "empty",
                std::path::Path::new("/usr"),
                std::path::Path::new("/run/user/1000"),
                None,
                |_| true,
                never,
            ),
            Response::Refused {
                problem: Problem::NothingToRun
            }
        );

        std::fs::create_dir_all(dir.path().join("has-one")).unwrap();
        std::fs::write(
            dir.path().join("has-one/bottle.toml"),
            "id = \"has-one\"\nprefix_root = \"/nowhere/pfx\"\negress = \"none\"\ngrants = []\n\
             program = [\"notepad.exe\"]\n",
        )
        .unwrap();

        // A bottle whose prefix was never made says so, rather than answering
        // with a pid for a process that is already gone.
        assert_eq!(
            launch(
                dir.path(),
                "has-one",
                std::path::Path::new("/usr"),
                std::path::Path::new("/run/user/1000"),
                None,
                |p| p != std::path::Path::new("/nowhere/pfx"),
                never,
            ),
            Response::Refused {
                problem: Problem::PrefixMissing
            }
        );

        // A machine with no Wine says so, rather than starting a confinement in
        // which the program is missing.
        assert_eq!(
            launch(
                dir.path(),
                "has-one",
                std::path::Path::new("/usr"),
                std::path::Path::new("/run/user/1000"),
                None,
                // The prefix is there; Wine is not. Ordered that way on purpose:
                // a missing prefix is checked first, so this closure has to leave
                // it in place or it would answer the earlier question.
                |p| p != std::path::Path::new("/usr/bin/wine"),
                never,
            ),
            Response::Refused {
                problem: Problem::NoWine
            }
        );
    }

    #[test]
    fn forgetting_moves_the_prefix_and_says_where() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::create_dir_all(dir.path().join("gone")).unwrap();
        std::fs::write(
            dir.path().join("gone/bottle.toml"),
            format!(
                "id = \"gone\"\nprefix_root = \"{}\"\negress = \"none\"\ngrants = []\n",
                prefix.display()
            ),
        )
        .unwrap();

        let moved = dir.path().join("trash/pfx");
        let answer = forget(dir.path(), "gone", |p| {
            std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
            std::fs::rename(p, &moved).unwrap();
            Ok(moved.clone())
        });
        assert_eq!(
            answer,
            Response::Forgotten {
                trashed_to: Some(moved.to_string_lossy().into_owned())
            },
            "a window can only tell somebody where their files went if this says it"
        );
        assert!(
            !dir.path().join("gone/bottle.toml").exists(),
            "the description is what makes the bottle exist"
        );
        assert!(
            moved.exists(),
            "the prefix moved rather than being destroyed"
        );
    }

    #[test]
    fn forgetting_something_that_is_not_there_says_which_kind_of_absent() {
        let dir = tempfile::tempdir().unwrap();
        let never = |_: &std::path::Path| -> Result<std::path::PathBuf, String> {
            panic!("nothing may be trashed for a bottle that was never found")
        };
        assert_eq!(
            forget(dir.path(), "../etc", never),
            Response::Refused {
                problem: Problem::BadId
            }
        );
        assert_eq!(
            forget(dir.path(), "absent", never),
            Response::Refused {
                problem: Problem::NoSuchBottle
            }
        );
    }

    #[test]
    fn a_request_survives_the_wire_it_will_travel_on() {
        // The tag is what a reader of a captured frame sees, so it is pinned here
        // rather than left to whatever serde happens to derive next.
        let asked = serde_json::to_string(&Request::Health { id: "steam".into() }).unwrap();
        assert_eq!(asked, r#"{"ask":"health","id":"steam"}"#);
        assert_eq!(
            serde_json::from_str::<Request>(&asked).unwrap(),
            Request::Health { id: "steam".into() }
        );

        let answered = serde_json::to_string(&Response::Refused {
            problem: Problem::NoSuchBottle,
        })
        .unwrap();
        assert_eq!(
            answered,
            r#"{"answer":"refused","problem":"no-such-bottle"}"#
        );
    }
}
