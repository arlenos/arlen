//! PAS-7: answering a schema's declared value source from this machine.
//!
//! The pure parsing and scanning lives in `settings-core`; this is the part that
//! actually touches the system - runs `pactl`, looks in the theme directory,
//! reads the installed desktop entries - and hands the text to it.
//!
//! Every source that cannot be consulted reports why. An audio server that is
//! not running produces `Unavailable("...")`, not an empty list: the page can
//! then say the devices could not be read, rather than telling the user they
//! have no audio devices.

use arlen_forage_recipe::settings::{SettingOption, ValueSource};
use arlen_settings_core::values::{
    browsers_in, locales_from, pactl_devices_from, resolve, themes_in, Resolution, SystemValues,
};

/// What the frontend gets back: either the choices or the reason there are none
/// to show. Kept as one shape with an explicit `available` flag so a page cannot
/// render a failed lookup as an empty dropdown by forgetting to check.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedOptions {
    /// Whether the source could be consulted at all.
    pub available: bool,
    /// The choices. Empty when `available` is false, and legitimately empty when
    /// it is true and the machine simply offers nothing.
    pub options: Vec<OptionView>,
    /// Why the source could not be consulted; empty when it could.
    pub reason: String,
}

/// One choice, as the page renders it.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionView {
    /// The value stored in the app's config.
    pub value: String,
    /// The name shown to the user.
    pub label: String,
    /// What choosing it means; often empty for machine-listed values.
    pub description: String,
}

impl From<Resolution> for ResolvedOptions {
    fn from(r: Resolution) -> Self {
        match r {
            Resolution::Options(options) => ResolvedOptions {
                available: true,
                options: options.into_iter().map(OptionView::from).collect(),
                reason: String::new(),
            },
            Resolution::Unavailable(reason) => ResolvedOptions {
                available: false,
                options: Vec::new(),
                reason,
            },
        }
    }
}

impl From<SettingOption> for OptionView {
    fn from(o: SettingOption) -> Self {
        OptionView {
            value: o.value,
            label: o.label,
            description: o.description,
        }
    }
}

/// This machine, as the resolver sees it.
/// The live machine, as the value sources see it.
pub struct ThisMachine;

impl ThisMachine {
    /// Run a listing command and hand its stdout to a parser, reporting the
    /// failure rather than an empty list when the command cannot be run.
    fn listing(program: &str, args: &[&str], parse: impl Fn(&str) -> Resolution) -> Resolution {
        match std::process::Command::new(program).args(args).output() {
            Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
            Ok(_) => Resolution::unavailable(format!("{program} could not list them")),
            Err(_) => Resolution::unavailable(format!("{program} is not installed")),
        }
    }
}

impl SystemValues for ThisMachine {
    fn audio_outputs(&self) -> Resolution {
        Self::listing("pactl", &["-f", "json", "list", "sinks"], pactl_devices_from)
    }

    fn audio_inputs(&self) -> Resolution {
        Self::listing("pactl", &["-f", "json", "list", "sources"], pactl_devices_from)
    }

    fn installed_themes(&self) -> Resolution {
        // The bundled ids are the two themes the system ships, resolved through
        // `sdk/theme` so this cannot drift from what the gallery offers.
        themes_in(
            &["dark", "light"],
            &arlen_theme::ArlenTheme::user_themes_dir(),
        )
    }

    fn locales(&self) -> Resolution {
        Self::listing("locale", &["-a"], locales_from)
    }

    fn browsers(&self) -> Resolution {
        browsers_in(&desktop_entry_dirs())
    }
}

/// Where installed desktop entries live, the user's own first so their entry
/// shadows a system one of the same name.
fn desktop_entry_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(std::path::PathBuf::from(&home).join(".local/share/applications"));
        dirs.push(std::path::PathBuf::from(home).join(".local/share/flatpak/exports/share/applications"));
    }
    dirs.push(std::path::PathBuf::from("/usr/share/applications"));
    dirs.push(std::path::PathBuf::from(
        "/var/lib/flatpak/exports/share/applications",
    ));
    dirs
}

/// Resolve a schema's declared value source against this machine.
///
/// The source is named, not described: the frontend passes the same closed
/// `ValueSource` the schema declared, so a page cannot ask for a source no
/// package could have named. An unknown name is refused rather than guessed.
#[tauri::command]
pub async fn settings_resolve_options(source: String) -> Result<ResolvedOptions, String> {
    let parsed: ValueSource = serde_json::from_value(serde_json::Value::String(source.clone()))
        .map_err(|_| format!("'{source}' is not a value source"))?;

    // The listings shell out and the theme scan touches the disk, so keep it off
    // the UI thread: a page that opens while PipeWire is wedged should render.
    tokio::task::spawn_blocking(move || ResolvedOptions::from(resolve(parsed, &ThisMachine)))
        .await
        .map_err(|e| format!("resolving the options failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The frontend names the source with the schema's own spelling, so the two
    /// cannot drift; anything else is refused rather than guessed at.
    #[test]
    fn only_a_declared_source_name_is_accepted() {
        let parse = |s: &str| {
            serde_json::from_value::<ValueSource>(serde_json::Value::String(s.to_string()))
        };
        assert_eq!(parse("audio_outputs").unwrap(), ValueSource::AudioOutputs);
        assert_eq!(
            parse("installed_themes").unwrap(),
            ValueSource::InstalledThemes
        );
        assert!(parse("audioOutputs").is_err());
        assert!(parse("/etc/shadow").is_err());
    }

    /// A failed lookup must not reach the page as an empty list, which reads as
    /// "you have none".
    #[test]
    fn an_unavailable_source_carries_its_reason() {
        let view = ResolvedOptions::from(Resolution::unavailable("pactl is not installed"));
        assert!(!view.available);
        assert!(view.options.is_empty());
        assert_eq!(view.reason, "pactl is not installed");

        let empty = ResolvedOptions::from(Resolution::Options(Vec::new()));
        assert!(empty.available, "an empty machine list is still an answer");
        assert!(empty.reason.is_empty());
    }
}
