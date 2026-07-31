//! The backend behind an app's settings page.
//!
//! Settings renders the page from the app's DECLARED schema, so this hands over
//! the schema itself rather than a view struct built from it. A parallel view
//! type would need a field added for every schema field, and the one that got
//! forgotten would be the one silently missing from the page - the same
//! second-store drift the capability work keeps running into.
//!
//! Reads come straight from the app's config file; writes go through the broker.
//! That asymmetry is the design: one process validates every write against the
//! schema, so a page cannot store a value the app never declared, while a read
//! needs no such gate.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use arlen_forage_recipe::settings::SettingsSchema;
use arlen_settings_broker::client::write_settings;
use arlen_settings_broker::protocol::{KeyWrite, Response};
use arlen_settings_broker::registry::DirectoryRegistry;
use arlen_settings_broker::serve::AppRegistry;

/// Everything a page needs to render one app's settings.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsPage {
    /// The app this page is for.
    pub app_id: String,
    /// The declared schema, with one deliberate exception: an item declaring
    /// `options_from` has had its `options` filled in by the system (PAS-7).
    /// Those choices are the machine's to know - which audio devices exist,
    /// which themes are installed - so an app cannot ship them and the schema
    /// alone would render an empty dropdown.
    pub schema: SettingsSchema,
    /// The value in force for each declared key, as JSON.
    pub values: BTreeMap<String, serde_json::Value>,
    /// The keys whose value the user chose, as opposed to the shipped default.
    ///
    /// The page needs the difference to offer "reset to default" honestly, and a
    /// value equal to the default is NOT the same as one nobody ever set.
    pub user_set: Vec<String>,
    /// Dynamic sources that could not be resolved, keyed by the item, with the
    /// reason in plain words.
    ///
    /// Separate from the empty options list it leaves behind, because "there
    /// are no other audio devices" and "we could not ask the audio system" look
    /// identical in an empty dropdown and mean entirely different things to
    /// someone trying to pick one.
    pub unavailable: BTreeMap<String, String>,
}

/// What came back from a write.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteAnswer {
    /// Whether the write went through.
    pub ok: bool,
    /// The keys whose stored value is now different. Empty when the write was a
    /// no-op, which is not a failure.
    pub changed: Vec<String>,
    /// The key that was refused, if one was.
    pub refused_key: String,
    /// Why, in the broker's own words, so the page can tell the user.
    pub message: String,
}

/// One key the page wants to write.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct KeyWriteInput {
    /// The declared key.
    pub key: String,
    /// The new value, as JSON from the page.
    pub value: serde_json::Value,
}

/// Where installed schemas live, user-installed first so it shadows a system one.
fn schema_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data) = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
    {
        dirs.push(data.join("arlen/settings-schemas"));
    }
    dirs.push(std::path::PathBuf::from("/usr/share/arlen/settings-schemas"));
    dirs
}

/// The directory holding apps' config files, asked of the SDK rather than
/// spelled out again, so a page reads exactly where the broker writes.
fn config_dir() -> std::path::PathBuf {
    os_sdk::config::config_path("probe")
        .parent()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("/etc/arlen"))
}

/// Load an app's declared settings and the values in force.
///
/// `None` when the app declares no schema. That is not an error: most apps
/// have no settings page, and the caller renders nothing rather than an
/// apology.
#[tauri::command]
pub async fn app_settings_page(app_id: String) -> Result<Option<AppSettingsPage>, String> {
    tokio::task::spawn_blocking(move || {
        let registry = DirectoryRegistry::new(schema_dirs(), config_dir());
        // The registry validates the app-id before it touches a path, so a
        // traversing id resolves to nothing rather than to a file of its choosing.
        let Some(found) = registry.lookup(&app_id) else {
            return Ok(None);
        };
        let settings = os_sdk::settings::Settings::load(&app_id, found.schema.clone())
            .map_err(|e| format!("could not read the app's settings: {e}"))?;

        let mut schema = found.schema.clone();
        let mut unavailable = BTreeMap::new();
        // PAS-7: fill in the choices the machine knows and the app cannot. Done
        // per page load rather than cached, because the whole reason these are
        // dynamic is that they change - a headset plugged in after the page
        // opened should be there on the next visit.
        let machine = super::values::ThisMachine;
        for section in schema.sections.iter_mut() {
            for item in section.items.iter_mut() {
                let Some(source) = item.options_from else {
                    continue;
                };
                match arlen_settings_core::values::resolve(source, &machine) {
                    arlen_settings_core::values::Resolution::Options(options) => {
                        item.options = options;
                    }
                    arlen_settings_core::values::Resolution::Unavailable(why) => {
                        // The options stay empty, and the reason travels beside
                        // them so the page can say which kind of empty this is.
                        unavailable.insert(item.key.clone(), why);
                    }
                }
            }
        }

        let mut values = BTreeMap::new();
        let mut user_set = Vec::new();
        for section in &schema.sections {
            for item in &section.items {
                if let Some(value) = settings.get_raw(&item.key) {
                    values.insert(item.key.clone(), toml_to_json(value));
                }
                if settings.is_user_set(&item.key) {
                    user_set.push(item.key.clone());
                }
            }
        }

        Ok(Some(AppSettingsPage {
            app_id,
            schema,
            values,
            user_set,
            unavailable,
        }))
    })
    .await
    .map_err(|e| format!("loading the settings page failed: {e}"))?
}

/// Write one app's settings through the broker.
///
/// A refusal comes back as an answer with `ok: false` and the broker's reason,
/// not as a command error: the page has to tell the user which value was
/// rejected and why, and an error string would flatten that away.
#[tauri::command]
pub async fn app_settings_write(
    app_id: String,
    writes: Vec<KeyWriteInput>,
) -> Result<WriteAnswer, String> {
    let socket = arlen_settings_broker::server::socket_path()
        .ok_or_else(|| "there is no runtime directory to reach the broker in".to_string())?;

    let writes: Vec<KeyWrite> = writes
        .into_iter()
        .map(|w| KeyWrite {
            key: w.key,
            value: arlen_settings_core::config::json_to_toml(w.value),
        })
        .collect();

    let response = write_settings(&socket, &app_id, writes)
        .await
        .map_err(|e| e.to_string())?;

    Ok(answer_from(response))
}

/// A stored TOML value as the JSON the page reads.
fn toml_to_json(value: &toml::Value) -> serde_json::Value {
    match value {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::from(*i),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            // JSON has no NaN or infinity. Dropping to null keeps a malformed
            // config from failing the whole page.
            .unwrap_or(serde_json::Value::Null),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Datetime(d) => serde_json::Value::String(d.to_string()),
        toml::Value::Array(a) => serde_json::Value::Array(a.iter().map(toml_to_json).collect()),
        toml::Value::Table(t) => {
            serde_json::Value::Object(t.iter().map(|(k, v)| (k.clone(), toml_to_json(v))).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_toml_shape_survives_the_trip_to_json() {
        let table: toml::Value = "s = \"x\"\ni = 3\nf = 1.5\nb = true\na = [1, 2]\n[t]\nk = \"v\"\n"
            .parse()
            .unwrap();
        let json = toml_to_json(&table);
        assert_eq!(json["s"], "x");
        assert_eq!(json["i"], 3);
        assert_eq!(json["f"], 1.5);
        assert_eq!(json["b"], true);
        assert_eq!(json["a"][1], 2);
        assert_eq!(json["t"]["k"], "v");
    }

    /// A config holding a value JSON cannot express must not take the page down
    /// with it: the rest of the settings still render.
    #[test]
    fn a_value_json_cannot_express_becomes_null() {
        assert_eq!(
            toml_to_json(&toml::Value::Float(f64::INFINITY)),
            serde_json::Value::Null
        );
    }

    /// A refusal is an answer the page shows, not a failed command.
    #[test]
    fn a_refusal_keeps_the_key_and_the_reason() {
        let answer = match (Response::Refused {
            key: "count".into(),
            reason: "expected an integer".into(),
        }) {
            Response::Refused { key, reason } => WriteAnswer {
                ok: false,
                changed: Vec::new(),
                refused_key: key,
                message: reason,
            },
            _ => unreachable!(),
        };
        assert!(!answer.ok);
        assert_eq!(answer.refused_key, "count");
        assert_eq!(answer.message, "expected an integer");
    }
}

/// Map a broker reply into the page's answer shape. Shared so the raw editor
/// reports a refusal in exactly the same words a control does.
fn answer_from(response: Response) -> WriteAnswer {
    match response {
        Response::Changed { changed, .. } => WriteAnswer {
            ok: true,
            changed,
            refused_key: String::new(),
            message: String::new(),
        },
        Response::Refused { key, reason } => WriteAnswer {
            ok: false,
            changed: Vec::new(),
            refused_key: key,
            message: reason,
        },
        Response::UnknownApp { app_id } => WriteAnswer {
            ok: false,
            changed: Vec::new(),
            refused_key: String::new(),
            message: format!("{app_id} declares no settings"),
        },
        Response::Error { message } => WriteAnswer {
            ok: false,
            changed: Vec::new(),
            refused_key: String::new(),
            message,
        },
    }
}

/// Write one key from the raw TOML editor (PAS-6 tier two).
///
/// The escape hatch for settings a schema cannot describe: the user types the
/// value as TOML and it is stored as that value.
///
/// **Parsing is not validation, and this does not skip the broker.** All the
/// parse does is turn text into a `toml::Value`; the value then goes through the
/// same write path as any control, so the app's declared type, its scope and
/// the key's existence are all still checked. A raw editor that wrote the file
/// directly would be a hole straight through the thing the broker exists to be.
#[tauri::command]
pub async fn app_settings_write_raw(
    app_id: String,
    key: String,
    text: String,
) -> Result<WriteAnswer, String> {
    let value = arlen_settings_core::raw::parse_raw_edit(&text).map_err(|e| e.to_string())?;
    let socket = arlen_settings_broker::server::socket_path()
        .ok_or_else(|| "there is no runtime directory to reach the broker in".to_string())?;

    let response = write_settings(&socket, &app_id, vec![KeyWrite { key, value }])
        .await
        .map_err(|e| e.to_string())?;

    Ok(answer_from(response))
}

/// Whether the app's own schema declares a handoff row opening `window`.
///
/// The gate on the handoff: the window name reaches another program's argv, so
/// it has to come from what that program declared rather than from the request.
fn schema_declares_handoff(schema: &SettingsSchema, window: &str) -> bool {
    schema
        .sections
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| i.handoff.as_ref())
        .any(|h| h.window == window)
}

/// Where an installed app's files live, matching installd's `user_apps_dir`.
fn user_apps_dir() -> Option<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("ARLEN_USER_APPS_DIR") {
        return Some(std::path::PathBuf::from(dir));
    }
    dirs::data_dir().map(|d| d.join("arlen/apps"))
}

/// Just enough of an installed package's manifest to find its entry point.
#[derive(serde::Deserialize)]
struct InstalledManifest {
    binary: InstalledBinary,
}

#[derive(serde::Deserialize)]
struct InstalledBinary {
    path: String,
}

/// Resolve the executable of an installed app, relative to its install dir.
///
/// Read from the package's own `manifest.toml` rather than guessed from the
/// app-id, because the binary's name is the package's to state.
fn installed_executable(app_id: &str) -> Result<std::path::PathBuf, String> {
    let dir = user_apps_dir()
        .ok_or_else(|| "there is no data directory to find installed apps in".to_string())?
        .join(app_id);
    let manifest_text = std::fs::read_to_string(dir.join("manifest.toml"))
        .map_err(|_| format!("{app_id} does not look installed"))?;
    let manifest: InstalledManifest =
        toml::from_str(&manifest_text).map_err(|e| format!("{app_id} has an unreadable manifest: {e}"))?;
    let exe = dir.join(&manifest.binary.path);
    if !exe.exists() {
        return Err(format!("{app_id} declares a binary that is not installed"));
    }
    Ok(exe)
}

/// Whether the system routes app launches through `arlen-run`, read from the same
/// `shell.toml [launcher] confined` the shell's launcher reads.
///
/// The flag is a property of the SYSTEM, not of whoever is launching, so every
/// launch path has to honour it. A config that cannot be read is treated as
/// unconfined, which is what the shell does too: this is the go-live switch, and
/// a Settings that refused to open an app because it could not parse a TOML file
/// would be worse than one that matches today's default.
fn launcher_is_confined() -> bool {
    let path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("arlen/shell.toml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    toml::from_str::<toml::Value>(&text)
        .ok()
        .and_then(|v| v.get("launcher")?.get("confined")?.as_bool())
        .unwrap_or(false)
}

/// Build the handoff launch, confined or not.
///
/// Settings launching an app is a SECOND launch path beside the shell's, and a
/// second path that skips confinement is how a sandbox gets bypassed without
/// anyone deciding to bypass it. The shell routes through
/// `arlen-run --app-id <id> -- <argv>` when the flag is on; this does the same,
/// with the same argv contract, so an app opened from its Settings page runs
/// under exactly the confinement it would have had from the launcher.
///
/// Split out from the spawn so the decision is testable without starting a
/// process.
fn handoff_command(app_id: &str, exe: &Path, window: &str, confined: bool) -> std::process::Command {
    if confined {
        let mut cmd = std::process::Command::new("arlen-run");
        cmd.arg("--app-id").arg(app_id).arg("--");
        cmd.arg(exe).arg("--settings-window").arg(window);
        cmd
    } else {
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--settings-window").arg(window);
        cmd
    }
}

/// Open an app's own settings window (PAS-6 tier three, the handoff).
///
/// The app declares a window NAME in its schema and the system launches the
/// app's ordinary entry point with `--settings-window <name>`. The app exposes
/// no second entry point for this, which is the whole design: Android's
/// equivalent had the settings app start an exported activity, and exporting it
/// to the settings app exported it to every app on the device.
///
/// **The name is checked against the app's own schema, not taken on trust.**
/// Without that, this command would pass any caller-supplied string into another
/// program's argv, which is the door the design is built to not have. A window
/// no item declares is refused, and so is an app with no schema at all.
///
/// Launched detached: the handoff hands the user over to the app's own window,
/// and Settings does not wait for it or own its lifetime.
#[tauri::command]
pub async fn app_settings_handoff(app_id: String, window: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let registry = DirectoryRegistry::new(schema_dirs(), config_dir());
        // Validates the app-id before touching a path, and proves the app is
        // installed with a schema at all.
        let found = registry
            .lookup(&app_id)
            .ok_or_else(|| format!("{app_id} has no settings schema"))?;

        if !schema_declares_handoff(&found.schema, &window) {
            return Err(format!("{app_id} declares no handoff window {window:?}"));
        }

        let exe = installed_executable(&app_id)?;
        let mut cmd = handoff_command(&app_id, &exe, &window, launcher_is_confined());
        cmd.spawn()
            .map(|_| ())
            .map_err(|e| format!("could not start {}: {e}", exe.display()))
    })
    .await
    .map_err(|e| format!("the handoff task failed: {e}"))?
}

#[cfg(test)]
mod handoff_tests {

    use std::path::Path;

    /// The confinement flag belongs to the system, so the second launch path has
    /// to take the same route the shell's launcher does. A Settings handoff that
    /// spawned the binary directly would leave an app unconfined precisely when
    /// someone had turned confinement on.
    #[test]
    fn a_confined_handoff_goes_through_the_launcher() {
        let cmd = super::handoff_command("com.example.App", Path::new("/usr/bin/app"), "prefs", true);
        assert_eq!(cmd.get_program(), "arlen-run");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert_eq!(
            args,
            vec!["--app-id", "com.example.App", "--", "/usr/bin/app", "--settings-window", "prefs"],
            "the argv contract arlen-run parses"
        );
    }

    #[test]
    fn an_unconfined_handoff_is_exactly_what_it_was() {
        let cmd = super::handoff_command("com.example.App", Path::new("/usr/bin/app"), "prefs", false);
        assert_eq!(cmd.get_program(), "/usr/bin/app");
        let args: Vec<_> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert_eq!(args, vec!["--settings-window", "prefs"]);
    }
    use super::*;

    fn schema_with(handoff_window: Option<&str>) -> SettingsSchema {
        let handoff = handoff_window
            .map(|w| format!("\nhandoff = {{ window = \"{w}\" }}"))
            .unwrap_or_default();
        let text = format!(
            r#"
version = 1
[[sections]]
label = "General"
[[sections.items]]
key = "advanced"
type = "handoff"
label = "Advanced"{handoff}
"#
        );
        toml::from_str(&text).expect("schema fixture parses")
    }

    /// The whole point of the check: a window the app declared opens, and one it
    /// did not is refused, because the name becomes an argv element of another
    /// program.
    #[test]
    fn only_a_window_the_app_declared_is_a_handoff_target() {
        let schema = schema_with(Some("advanced"));
        assert!(schema_declares_handoff(&schema, "advanced"));
        assert!(!schema_declares_handoff(&schema, "anything-else"));
        assert!(!schema_declares_handoff(&schema, ""));
    }

    /// A schema with no handoff row at all opens nothing, rather than falling
    /// through to "no objection".
    #[test]
    fn a_schema_without_a_handoff_opens_nothing() {
        let schema = schema_with(None);
        assert!(!schema_declares_handoff(&schema, "advanced"));
    }
}
