/// Quick Settings panel state, customisation, and tile registry.
///
/// `layout`   — `~/.config/arlen/quicksettings.toml` schema and writers.
/// `status`   — Generic `arlen://qs/status/<channel>` event-bus
///              infrastructure that backends publish onto and tiles
///              subscribe to.
/// `defaults` — Bundled tile-default catalogue used as the first-run
///              fallback when the user file is missing or empty.

pub mod defaults;
pub mod layout;
pub mod status;
