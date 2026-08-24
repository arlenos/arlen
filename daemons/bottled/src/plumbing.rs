//! What a bottle needs from the host that is not one of its grants.
//!
//! A grant is a directory the person chose. Plumbing is everything else a program
//! needs to be a program at all: a display to draw on, the font configuration that
//! tells it what a font is, a GPU node if it was allowed one. None of it belongs in
//! the drive table, and none of it should be bound because a program might one day
//! want it. So this is a list with a reason and a switch on each line, and nothing
//! is bound that the host does not actually have.
//!
//! The display entry is the one with history. X11 clients look for their socket at
//! the hard-coded path `/tmp/.X11-unix`, and a confined app's `/tmp` is a private
//! tmpfs, so that bind has to be applied after the mask or the program finds no
//! socket and reports no display. The confiner does that routing now; this module
//! only has to ask for the bind.

use std::path::{Path, PathBuf};

use arlen_confiner::Bind;
use serde::{Deserialize, Serialize};

/// How the bottle reaches a screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Display {
    /// Wine's X11 driver under XWayland, which `wine-proton-plan.md` picked as the
    /// path that works today. The socket directory is bound whole rather than the
    /// one socket: an X client resolves `DISPLAY=:N` to a filename inside it, and
    /// which N the compositor hands out is not this module's to predict.
    X11,
    /// Wine's native Wayland driver, still experimental upstream. One socket, named
    /// by `WAYLAND_DISPLAY`, under the runtime directory.
    Wayland(String),
    /// The program draws nothing. Not a degenerate case: an installer run with
    /// `/S` and a command-line tool both work here, and giving them a display is
    /// giving them a way to be seen doing something nobody asked for.
    None,
}

/// What a bottle was allowed beyond its drives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plumbing {
    /// How it draws.
    pub display: Display,
    /// Whether it may use the GPU. Off by default: DXVK and VKD3D need `/dev/dri`,
    /// and a program that renders in software does not, so this follows the
    /// program rather than the machine.
    pub gpu: bool,
    /// Whether the host's font configuration is readable. Wine without it falls
    /// back to a built-in font and complains three times per launch, which is
    /// survivable for a console program and not for anything that draws text.
    pub fonts: bool,
}

impl Default for Plumbing {
    /// Nothing. A bottle is deny-by-default in this axis as in the others.
    fn default() -> Self {
        Plumbing {
            display: Display::None,
            gpu: false,
            fonts: false,
        }
    }
}

/// The X11 socket directory. Hard-coded in libX11, not configurable by an
/// environment variable, which is why it cannot simply be moved out of the masked
/// `/tmp`.
pub const X11_SOCKET_DIR: &str = "/tmp/.X11-unix";

/// The binds the plumbing asks for, dropping any whose source is not on this host.
///
/// `exists` is injected rather than called directly so the list can be tested
/// against a machine that is not this one. Binding a source bwrap cannot find makes
/// the whole launch fail, so a missing font configuration has to mean no font
/// configuration, not no program.
pub fn plumbing_binds(
    plumbing: &Plumbing,
    runtime_dir: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Bind> {
    let mut wanted: Vec<Bind> = Vec::new();

    match &plumbing.display {
        Display::X11 => {
            let p = X11_SOCKET_DIR.to_string();
            // Read-write: an X client writes to its socket.
            wanted.push(Bind::ReadWrite(p.clone(), p));
        }
        Display::Wayland(name) => {
            let p = if Path::new(name).is_absolute() {
                PathBuf::from(name)
            } else {
                runtime_dir.join(name)
            };
            let p = p.to_string_lossy().to_string();
            wanted.push(Bind::ReadWrite(p.clone(), p));
        }
        Display::None => {}
    }

    if plumbing.gpu {
        wanted.push(Bind::ReadWrite("/dev/dri".into(), "/dev/dri".into()));
    }

    if plumbing.fonts {
        // The configuration only. The font FILES live under /usr/share/fonts, which
        // the read-only /usr already covers, so this adds the rules and not the
        // fonts.
        wanted.push(Bind::ReadOnly("/etc/fonts".into(), "/etc/fonts".into()));
    }

    wanted
        .into_iter()
        .filter(|b| {
            let src = match b {
                Bind::ReadOnly(s, _) | Bind::ReadWrite(s, _) => s,
            };
            exists(Path::new(src))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(_: &Path) -> bool {
        true
    }

    #[test]
    fn a_bottle_that_draws_nothing_asks_for_nothing() {
        assert_eq!(
            plumbing_binds(&Plumbing::default(), Path::new("/run/user/1000"), all),
            vec![]
        );
    }

    #[test]
    fn the_x_socket_directory_is_asked_for_whole() {
        let p = Plumbing {
            display: Display::X11,
            ..Default::default()
        };
        assert_eq!(
            plumbing_binds(&p, Path::new("/run/user/1000"), all),
            vec![Bind::ReadWrite(
                X11_SOCKET_DIR.into(),
                X11_SOCKET_DIR.into()
            )]
        );
    }

    #[test]
    fn a_wayland_socket_is_named_under_the_runtime_directory() {
        let p = Plumbing {
            display: Display::Wayland("wayland-0".into()),
            ..Default::default()
        };
        assert_eq!(
            plumbing_binds(&p, Path::new("/run/user/1000"), all),
            vec![Bind::ReadWrite(
                "/run/user/1000/wayland-0".into(),
                "/run/user/1000/wayland-0".into()
            )]
        );
    }

    #[test]
    fn an_absolute_wayland_display_is_taken_as_it_stands() {
        let p = Plumbing {
            display: Display::Wayland("/tmp/wl.sock".into()),
            ..Default::default()
        };
        assert_eq!(
            plumbing_binds(&p, Path::new("/run/user/1000"), all),
            vec![Bind::ReadWrite(
                "/tmp/wl.sock".into(),
                "/tmp/wl.sock".into()
            )]
        );
    }

    #[test]
    fn what_the_host_does_not_have_is_not_asked_for() {
        // bwrap fails the whole launch on a missing bind source, so a machine with
        // no GPU node has to mean a bottle without one, not a bottle that will not
        // start.
        let p = Plumbing {
            display: Display::X11,
            gpu: true,
            fonts: true,
        };
        let binds = plumbing_binds(&p, Path::new("/run/user/1000"), |path| {
            path != Path::new("/dev/dri")
        });
        assert!(!binds
            .iter()
            .any(|b| matches!(b, Bind::ReadWrite(s, _) if s == "/dev/dri")));
        assert_eq!(binds.len(), 2);
    }

    #[test]
    fn the_gpu_is_off_unless_it_was_asked_for() {
        let p = Plumbing {
            display: Display::X11,
            ..Default::default()
        };
        let binds = plumbing_binds(&p, Path::new("/run/user/1000"), all);
        assert!(!binds
            .iter()
            .any(|b| matches!(b, Bind::ReadWrite(s, _) if s == "/dev/dri")));
    }
}
