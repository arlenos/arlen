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
    /// The drive letters it was granted, in letter order. Derived through the
    /// same mapping the launcher runs, so the panel and the bottle cannot
    /// disagree about which letter is which.
    pub drives: Vec<char>,
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
            .map(|drives| drives.iter().map(|d| d.letter).collect())
            .unwrap_or_default(),
    }
}

/// Answer one request against a bottles directory.
///
/// Pure over the filesystem it is handed, so the whole vocabulary is testable
/// without a socket, a peer or a running Wine.
pub fn handle_request(bottles_dir: &Path, request: &Request) -> Response {
    match request {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_machine_lists_no_bottles() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            handle_request(dir.path(), &Request::ListBottles),
            Response::Bottles {
                bottles: vec![],
                unreadable: vec![]
            },
            "a machine with no bottles has none, which is not a failure to read them"
        );
    }

    #[test]
    fn a_bottle_that_will_not_read_is_named_rather_than_dropped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("broken")).unwrap();
        std::fs::write(dir.path().join("broken/bottle.toml"), "id = [not toml").unwrap();

        let Response::Bottles {
            bottles,
            unreadable,
        } = handle_request(dir.path(), &Request::ListBottles)
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
            Response::Refused {
                problem: Problem::BadId
            }
        );
        assert_eq!(
            handle_request(
                dir.path(),
                &Request::Health {
                    id: "not-here".into()
                }
            ),
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
