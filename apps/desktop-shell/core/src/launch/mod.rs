//! Launching: deciding which program runs, and running it the one confined way.
//!
//! There are three places in the tree that start a third-party application
//! today - the shell launcher, the per-app Settings handoff, and the portal's
//! `xdg-open` - and each decides on its own whether to route through
//! `arlen-run`. Two of them read `shell.toml [launcher] confined`; the third
//! does not, and cannot usefully be taught to, because it is `xdg-open` that
//! resolves a URI to a handler and `arlen-run` needs the app id that resolution
//! produces. Nobody holds both halves, which is exactly how the gap opened.
//!
//! So resolution and launch belong together: the component that decides WHICH
//! program runs is the component that launches it, and it is the only place
//! either an app id or a URI ends in a process. This module is that component's
//! testable half; the spawning stays in the host, which has the config and the
//! process machinery.
//!
//! [`mimeapps`] is the resolution: a URI's MIME type to the desktop id that
//! handles it, per the freedesktop association spec.

pub mod exec;
pub mod mimeapps;
pub mod search;
