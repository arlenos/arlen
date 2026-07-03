//! AI layer status command (Phase 9-α S7).
//!
//! The AI daemon and proxy are D-Bus services, not socket daemons,
//! so liveness is probed by asking the session bus whether their
//! well-known names currently have an owner. This is the D-Bus
//! analogue of the socket-existence checks the About / Knowledge
//! pages use.
//!
//! The `enabled` / `provider` settings are not read here: the AI
//! page already gets those through the generic `ai.toml` config
//! store. This command answers only "is the daemon process alive".

use std::time::Duration;

use serde::Serialize;

/// AI daemon name on the session bus.
const AI_DAEMON_NAME: &str = "org.arlen.AI1";
/// AI daemon object path.
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";
/// AI proxy name on the session bus.
const AI_PROXY_NAME: &str = "org.arlen.AIProxy1";
/// Upper bound on the explanation call; the daemon reads the graph and calls
/// the provider, so allow a generous window but never hang the page.
const EXPLAIN_TIMEOUT: Duration = Duration::from_secs(90);

/// Liveness of the AI layer's two daemons.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    /// `org.arlen.AI1` has an owner on the session bus.
    pub daemon_running: bool,
    /// `org.arlen.AIProxy1` has an owner on the session bus.
    pub proxy_running: bool,
}

/// Probe whether the AI daemon and proxy are running.
#[tauri::command]
pub async fn ai_status() -> Result<AiStatus, String> {
    let connection = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            // No session bus at all — report both as down rather
            // than failing the command, so the page still renders.
            log::warn!("[ai] session bus unavailable: {e}");
            return Ok(AiStatus {
                daemon_running: false,
                proxy_running: false,
            });
        }
    };
    let dbus = zbus::fdo::DBusProxy::new(&connection)
        .await
        .map_err(|e| format!("DBusProxy: {e}"))?;

    Ok(AiStatus {
        daemon_running: name_has_owner(&dbus, AI_DAEMON_NAME).await,
        proxy_running: name_has_owner(&dbus, AI_PROXY_NAME).await,
    })
}

/// Ask the AI daemon for a plain-language summary of what the computer is
/// doing right now (Foundation §5.8 System Explanation Mode). A single bounded
/// D-Bus call to `org.arlen.AI1`; errors (daemon down, disabled, insufficient
/// scope, timeout) come back as a readable string the page shows.
#[tauri::command]
pub async fn ai_explain() -> Result<String, String> {
    let connection = zbus::Connection::session()
        .await
        .map_err(|e| format!("session bus: {e}"))?;
    let proxy = zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME)
        .await
        .map_err(|e| format!("ai daemon unavailable: {e}"))?;
    match tokio::time::timeout(
        EXPLAIN_TIMEOUT,
        proxy.call::<_, _, String>("explain_system", &()),
    )
    .await
    {
        Ok(Ok(summary)) => Ok(summary),
        Ok(Err(zbus::Error::MethodError(_, detail, _))) => {
            Err(detail.unwrap_or_else(|| "explanation failed".to_string()))
        }
        Ok(Err(e)) => Err(format!("explanation failed: {e}")),
        Err(_) => Err("the explanation timed out".to_string()),
    }
}

/// Call a String-returning member on the AI daemon, returning `fallback` on any
/// connection or call failure (the manager reads are advisory - a down daemon
/// shows an empty surface rather than erroring the page).
async fn ai_call_string(member: &str, fallback: &str) -> String {
    let Ok(connection) = zbus::Connection::session().await else {
        return fallback.to_string();
    };
    let Ok(proxy) =
        zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME).await
    else {
        return fallback.to_string();
    };
    proxy
        .call::<_, _, String>(member, &())
        .await
        .unwrap_or_else(|_| fallback.to_string())
}

/// The catalogued providers for the Settings AI-providers manager
/// (`ai_providers_list`): a JSON array of `{ id, name, kind, enabled,
/// configured, status }`. Empty array if the daemon is unreachable.
#[tauri::command]
pub async fn ai_providers_list() -> String {
    ai_call_string("ai_providers_list", "[]").await
}

/// The configured default provider/model + ranked fallback (`ai_defaults_get`),
/// as `{ provider, model, ranking }`, for the manager's Default-Models page.
/// Empty object if the daemon is unreachable.
#[tauri::command]
pub async fn ai_defaults_get() -> String {
    ai_call_string("ai_defaults_get", "{}").await
}

/// The model catalog for the Settings Default-Models page (`ai_models_list`): a
/// JSON array of `{ provider, model, contextWindow, kind, available }`, the same
/// catalog the harness picker reads. The page pairs it with `ai_defaults_get`/
/// `ai_defaults_set` to choose the default; empty array if the daemon is
/// unreachable.
#[tauri::command]
pub async fn ai_models_list() -> String {
    ai_call_string("ai_models_list", "[]").await
}

/// Enable or disable a catalogued provider (`ai_provider_set_enabled`). Returns
/// the daemon's `ok` / `error: ...` status; a transport failure maps to an
/// `error:` string so the manager surfaces it.
#[tauri::command]
pub async fn ai_provider_set_enabled(id: String, enabled: bool) -> String {
    let Ok(connection) = zbus::Connection::session().await else {
        return "error: session bus unavailable".to_string();
    };
    let Ok(proxy) =
        zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME).await
    else {
        return "error: AI daemon unavailable".to_string();
    };
    proxy
        .call::<_, _, String>("ai_provider_set_enabled", &(id.as_str(), enabled))
        .await
        .unwrap_or_else(|e| format!("error: {e}"))
}

/// Test a catalogued provider's connectivity (`ai_provider_test`). Returns the
/// daemon's verdict JSON `{ ok, httpStatus?, network? }`; the daemon GETs the
/// provider's catalogued model-list endpoint through the proxy (no caller URL,
/// so no egress-consent step). A transport failure maps to a `network` verdict
/// so the manager always gets the uniform shape.
#[tauri::command]
pub async fn ai_provider_test(id: String) -> String {
    let network = |reason: &str| format!(r#"{{"ok":false,"network":"{reason}"}}"#);
    let Ok(connection) = zbus::Connection::session().await else {
        return network("session bus unavailable");
    };
    let Ok(proxy) =
        zbus::Proxy::new(&connection, AI_DAEMON_NAME, AI_OBJECT_PATH, AI_DAEMON_NAME).await
    else {
        return network("AI daemon unavailable");
    };
    proxy
        .call::<_, _, String>("ai_provider_test", &(id.as_str(),))
        .await
        .unwrap_or_else(|_| network("test failed"))
}

async fn name_has_owner(dbus: &zbus::fdo::DBusProxy<'_>, name: &str) -> bool {
    let Ok(bus_name) = zbus::names::BusName::try_from(name) else {
        return false;
    };
    dbus.name_has_owner(bus_name).await.unwrap_or(false)
}

/// Local-hardware probe for the Models hub (`ai_hardware_probe`): total RAM, the
/// accelerator kind (APU vs discrete GPU) and its VRAM, plus a plain one-line
/// summary of what fits at a good speed. Computed locally from
/// `arlen-ai-model-manager` (no daemon round-trip; hardware detection is a pure
/// local read), so the hub's fit line is real instead of a fixture.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    /// Total system RAM in GB.
    ram_gb: f64,
    /// `"apu"` (unified memory) or `"discrete"` (dedicated VRAM).
    accelerator: String,
    /// Dedicated VRAM in GB for a discrete GPU; `null` for an APU.
    vram_gb: Option<f64>,
    /// A plain sentence: the largest model size that runs well here.
    summary: String,
}

#[tauri::command]
pub fn ai_hardware_probe() -> HardwareInfo {
    use arlen_ai_model_manager as mm;
    let hw = mm::detect_hardware();
    let (accelerator, vram_gb) = match hw.accelerator {
        mm::Accelerator::Apu => ("apu".to_string(), None),
        mm::Accelerator::Discrete { vram_gib } => ("discrete".to_string(), Some(vram_gib)),
    };
    HardwareInfo {
        ram_gb: hw.ram_gib,
        accelerator,
        vram_gb,
        summary: hardware_summary(&hw),
    }
}

/// A plain one-line capability summary: the largest common model size that FITS
/// (not merely may-slow) at the Q4_K_M default, phrased for the hardware kind. On
/// an APU the speed axis leads (the plan), so a fits-but-slow size is not claimed.
fn hardware_summary(hw: &arlen_ai_model_manager::Hardware) -> String {
    use arlen_ai_model_manager as mm;
    const SIZES_B: [f64; 7] = [1.0, 3.0, 7.0, 8.0, 13.0, 34.0, 70.0];
    let best = SIZES_B
        .iter()
        .rev()
        .copied()
        .find(|&b| mm::fit_badge(b, mm::Quant::Q4KM, hw) == mm::FitBadge::Fits);
    match best {
        Some(b) => format!(
            "Your machine can run models up to about {}B at a good speed.",
            b as u32
        ),
        None => "Your machine is best with small (1B) models.".to_string(),
    }
}

/// One curated catalogue model for the Models hub (`ai_models_catalog`), with the
/// per-hardware fit resolved. Matches the store's `Model` shape (camelCase).
#[derive(serde::Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    /// Stable id, `local/<slug>`.
    id: String,
    /// Display name.
    name: String,
    /// Always `"local"` for a curated on-device model.
    provider: String,
    /// Always `"local"`.
    kind: String,
    /// Task groups this model is listed under (never empty: General is the default).
    tasks: Vec<String>,
    /// Parameter count in billions.
    params_b: f64,
    /// `"fits"` / `"may-be-slow"` / `"wont-fit"` on this machine.
    fit: String,
    /// Estimated generation speed in tokens/sec.
    tokens_per_sec: f64,
    /// Resident size in GiB at the best-fitting quant.
    size_gb: f64,
    /// Whether a GGUF for this model is already on disk.
    installed: bool,
    /// Whether it is the baked (system-store) default, which cannot be removed.
    baked: bool,
    /// Curated entries are never disk-imports.
    imported: bool,
    /// The advanced (uncensored) door set.
    advanced: bool,
}

/// The curated model catalogue with fit/size/speed resolved for this machine
/// (`ai_models_catalog`). The curation lives in `arlen-ai-model-manager`'s bundled
/// TOML; this resolves the per-hardware fit and marries it to what is on disk.
#[tauri::command]
pub fn ai_models_catalog() -> Vec<CatalogModel> {
    use arlen_ai_model_manager as mm;
    let hw = mm::detect_hardware();
    let installed = mm::installed::installed_models();
    let catalog = mm::bundled_catalog().unwrap_or_default();
    catalog_to_models(&catalog, &hw, &installed)
}

fn task_str(task: arlen_ai_model_manager::Task) -> &'static str {
    use arlen_ai_model_manager::Task;
    match task {
        Task::General => "general",
        Task::Writing => "writing",
        Task::Coding => "coding",
        Task::Reasoning => "reasoning",
    }
}

/// Pure mapping from the curated catalogue to the hub's model list, resolving the
/// best-fitting quant, the fit badge, the resident size and an estimated speed for
/// `hw`, and marrying each entry to `installed` (on-disk) state. Split out so the
/// fit resolution is unit-tested without probing real hardware or the filesystem.
fn catalog_to_models(
    catalog: &arlen_ai_model_manager::Catalog,
    hw: &arlen_ai_model_manager::Hardware,
    installed: &[arlen_ai_model_manager::installed::InstalledModel],
) -> Vec<CatalogModel> {
    use arlen_ai_model_manager as mm;
    catalog
        .models
        .iter()
        .map(|m| {
            let quant = mm::best_fitting_quant(m.params_b, hw).unwrap_or(mm::Quant::Q4KM);
            let fit = match mm::fit_badge(m.params_b, quant, hw) {
                mm::FitBadge::Fits => "fits",
                mm::FitBadge::MaySlow => "may-be-slow",
                mm::FitBadge::WontFit => "wont-fit",
            };
            let size_gb = mm::footprint_gib(m.params_b, quant);
            let tokens_per_sec =
                mm::estimate_tokens_per_sec(mm::weights_gib(m.params_b, quant), hw.mem_bandwidth_gbps);
            // Marry to disk: the GGUF file this source+quant resolves to.
            let file = mm::download::gguf_filename(&m.source, quant);
            let on_disk = file
                .as_ref()
                .and_then(|f| installed.iter().find(|im| &im.file_name == f));
            let baked = on_disk.is_some_and(|im| im.location == mm::installed::ModelLocation::System);
            let tasks: Vec<String> = if m.tasks.is_empty() {
                vec!["general".to_string()]
            } else {
                m.tasks.iter().map(|t| task_str(*t).to_string()).collect()
            };
            CatalogModel {
                id: format!("local/{}", m.name.to_lowercase()),
                name: m.name.clone(),
                provider: "local".to_string(),
                kind: "local".to_string(),
                tasks,
                params_b: m.params_b,
                fit: fit.to_string(),
                tokens_per_sec,
                size_gb,
                installed: on_disk.is_some(),
                baked,
                imported: false,
                advanced: m.advanced,
            }
        })
        .collect()
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use arlen_ai_model_manager as mm;

    #[test]
    fn catalog_maps_with_fit_and_installed_state() {
        let catalog: mm::Catalog = mm::parse_catalog(
            "[[model]]\nname = \"Tiny-1B\"\nparams_b = 1.0\ntasks = [\"general\"]\nsource = \"bartowski/Tiny-1B-Instruct-GGUF\"\n",
        )
        .unwrap();
        // A comfortable APU: 32 GiB, generous bandwidth -> a 1B model fits.
        let hw = mm::Hardware {
            ram_gib: 32.0,
            accelerator: mm::Accelerator::Apu,
            mem_bandwidth_gbps: 100.0,
        };
        let out = catalog_to_models(&catalog, &hw, &[]);
        assert_eq!(out.len(), 1);
        let m = &out[0];
        assert_eq!(m.id, "local/tiny-1b");
        assert_eq!(m.provider, "local");
        assert_eq!(m.fit, "fits");
        assert!(m.size_gb > 0.0);
        assert!(m.tokens_per_sec > 0.0);
        assert!(!m.installed, "nothing on disk in this test");
        assert!(!m.baked);
        assert_eq!(m.tasks, vec!["general".to_string()]);
    }
}

/// The result of removing a local model (`ai_local_models_delete`): the bytes
/// reclaimed, so the hub can update the free-space line.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteResult {
    /// Bytes freed by removing the model file.
    freed_bytes: u64,
}

/// Remove an installed local model by id (`ai_local_models_delete`). Resolves the
/// id to its on-disk GGUF via `arlen-ai-model-manager::installed`, refuses to
/// remove the baked system default (`ModelLocation::System`), and deletes the
/// user-store file. Errors if the id is unknown or the file cannot be removed.
#[tauri::command]
pub fn ai_local_models_delete(id: String) -> Result<DeleteResult, String> {
    use arlen_ai_model_manager as mm;
    let id = id.strip_prefix("local/").unwrap_or(&id);
    let installed = mm::installed::installed_models();
    let model = mm::installed::find_by_id(id, &installed)
        .ok_or_else(|| format!("no installed model with id '{id}'"))?;
    if model.location == mm::installed::ModelLocation::System {
        return Err("the baked default model cannot be removed".to_string());
    }
    let freed_bytes = model.size_bytes;
    std::fs::remove_file(&model.path).map_err(|e| format!("could not remove model: {e}"))?;
    Ok(DeleteResult { freed_bytes })
}

/// A model imported from disk for the Models hub (`ai_local_models_import`). The
/// required fields of the store's `Model`; sizing/fit metadata is omitted (a GGUF
/// header parse for params/context is a follow-up), so the hub shows it as an
/// installed local model without a fit badge.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedModel {
    /// Stable id, `local/<stem>`.
    id: String,
    /// Display name (the file stem).
    name: String,
    /// Always `"local"`.
    provider: String,
    /// Always `"local"`.
    kind: String,
    /// Task groups (General by default).
    tasks: Vec<String>,
    /// Always true (it is now on disk).
    installed: bool,
    /// Never the baked default.
    baked: bool,
    /// Always true.
    imported: bool,
    /// Never advanced.
    advanced: bool,
}

/// Import a GGUF model from disk (`ai_local_models_import`): open a file picker,
/// validate the GGUF magic, copy it into the user model store, and return it as a
/// selectable local model. Errors if nothing was picked or the file is not a valid
/// GGUF; the frontend distinguishes a real import from a cancel by the error.
#[tauri::command]
pub async fn ai_local_models_import() -> Result<ImportedModel, String> {
    use arlen_ai_model_manager as mm;
    let src = crate::commands::picker::pick_gguf_file()
        .await
        .ok_or_else(|| "no file selected".to_string())?;
    let store = mm::download::model_store_dir()
        .ok_or_else(|| "model store directory is unavailable".to_string())?;
    let installed = mm::installed::import_gguf(std::path::Path::new(&src), &store)?;
    let id = mm::installed::model_id(&installed);
    Ok(ImportedModel {
        id: format!("local/{id}"),
        name: id,
        provider: "local".to_string(),
        kind: "local".to_string(),
        tasks: vec!["general".to_string()],
        installed: true,
        baked: false,
        imported: true,
        advanced: false,
    })
}
