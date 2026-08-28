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
        Request::Launch { .. } | Request::Forget { .. } | Request::Runtimes => return None,
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
