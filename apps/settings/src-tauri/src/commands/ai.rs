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
use tauri::{AppHandle, Emitter};

/// AI daemon name on the session bus.
const AI_DAEMON_NAME: &str = "org.arlen.AI1";
/// AI daemon object path.
const AI_OBJECT_PATH: &str = "/org/arlen/AI1";
/// AI proxy name on the session bus.
const AI_PROXY_NAME: &str = "org.arlen.AIProxy1";
/// The AI agent daemon (behaviours/skills surface), distinct from the AI daemon.
const AGENT_NAME: &str = "org.arlen.AIAgent1";
/// The AI agent daemon's object path.
const AGENT_OBJECT_PATH: &str = "/org/arlen/AIAgent1";
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

/// Call a String-returning member on the AI AGENT daemon (`org.arlen.AIAgent1`),
/// returning `fallback` on any connection or call failure. Distinct from
/// [`ai_call_string`] (the AI daemon `org.arlen.AI1`): the agent owns the
/// behaviour/skill surface, so a down agent shows an empty list, not an error.
async fn agent_call_string(member: &str, fallback: &str) -> String {
    let Ok(connection) = zbus::Connection::session().await else {
        return fallback.to_string();
    };
    let Ok(proxy) = zbus::Proxy::new(&connection, AGENT_NAME, AGENT_OBJECT_PATH, AGENT_NAME).await
    else {
        return fallback.to_string();
    };
    proxy
        .call::<_, _, String>(member, &())
        .await
        .unwrap_or_else(|_| fallback.to_string())
}

/// The AI agent's loaded behaviours for the Settings AI-behaviours panel: a JSON
/// array of `{ name, description, whenToUse, kind, enabled }` from the agent's
/// `list_skills`, read live from the configured sources so it is the same set the
/// daemon would act on. Identity and routing hints only, never a behaviour body
/// or user data. Empty array if the agent is unreachable (the panel then shows
/// no behaviours rather than erroring).
#[tauri::command]
pub async fn ai_behaviours() -> String {
    agent_call_string("list_skills", "[]").await
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
    /// Parameter count in billions, from the GGUF header when it records it.
    #[serde(skip_serializing_if = "Option::is_none")]
    params_b: Option<f64>,
    /// The model architecture (`general.architecture`, e.g. `"llama"`, `"qwen2"`)
    /// from the GGUF header, when it records it.
    #[serde(skip_serializing_if = "Option::is_none")]
    architecture: Option<String>,
    /// The trained context window in tokens (`<arch>.context_length`), when the
    /// file records it - the Models hub surfaces it so a user knows the ceiling.
    #[serde(skip_serializing_if = "Option::is_none")]
    context_length: Option<u64>,
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
    // Best-effort GGUF header metadata: a real name + parameter count when the
    // file records them. A parse failure leaves the file-stem name and no params.
    let meta = mm::gguf::read_gguf_metadata(&installed.path).ok();
    let name = meta
        .as_ref()
        .and_then(|m| m.name.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| id.clone());
    let params_b = meta
        .as_ref()
        .and_then(|m| m.parameter_count)
        .map(|p| p as f64 / 1e9);
    let architecture = meta
        .as_ref()
        .and_then(|m| m.architecture.clone())
        .filter(|a| !a.trim().is_empty());
    let context_length = meta.as_ref().and_then(|m| m.context_length);
    Ok(ImportedModel {
        id: format!("local/{id}"),
        name,
        provider: "local".to_string(),
        kind: "local".to_string(),
        tasks: vec!["general".to_string()],
        params_b,
        architecture,
        context_length,
        installed: true,
        baked: false,
        imported: true,
        advanced: false,
    })
}

/// Search Hugging Face for installable GGUF models (`ai_models_search_hf`). An
/// OPT-IN egress: the hub calls this only on an explicit search action, never on
/// keystroke. Runs the SSRF-pinned GET off the async runtime via `spawn_blocking`
/// and returns the hits (`{ id, downloads, likes }`, most-downloaded first);
/// `limit` defaults to 20, clamped to 50 by the search core. A network / blocked /
/// bad-status error is surfaced as a string so the hub can show it.
#[tauri::command]
pub async fn ai_models_search_hf(
    query: String,
    limit: Option<u32>,
) -> Result<Vec<arlen_ai_model_manager::hf::HfHit>, String> {
    let limit = limit.unwrap_or(20);
    tokio::task::spawn_blocking(move || arlen_ai_model_manager::hf::search_hf(&query, limit))
        .await
        .map_err(|e| format!("search task failed: {e}"))?
        .map_err(|e| e.to_string())
}

/// The progress event payload emitted as a model download streams
/// (`ai:model-download-progress`). `total_bytes` is 0 until the server's size is
/// known.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadProgress {
    id: String,
    bytes_fetched: u64,
    total_bytes: u64,
}

/// Download a curated catalog model into the user model store
/// (`ai_local_models_download`). Resolves the entry's source repo + the
/// best-fitting quant for this machine, resolves the file's sha256 from the HF
/// tree, then fetches the GGUF over the SSRF-safe egress with sha verification,
/// emitting `ai:model-download-progress` events as it streams. This is the one
/// consented outbound: a Settings model download is user-initiated, so the
/// explicit confirmed click is the consent (routing through the consent broker is
/// a follow-up once its async decision-return lands). Errors carry the failure.
#[tauri::command]
pub async fn ai_local_models_download(
    app: AppHandle,
    cancels: tauri::State<'_, DownloadCancels>,
    id: String,
) -> Result<(), String> {
    use arlen_ai_model_manager as mm;

    // Resolve the catalog entry -> source repo + best-fitting quant -> url/dest/file.
    let slug = id.strip_prefix("local/").unwrap_or(&id).to_ascii_lowercase();
    let catalog = mm::bundled_catalog().map_err(|e| format!("catalog: {e}"))?;
    let hw = mm::detect_hardware();
    let model = catalog
        .models
        .into_iter()
        .find(|m| m.name.to_ascii_lowercase() == slug)
        .ok_or_else(|| format!("unknown model: {id}"))?;
    let quant = mm::best_fitting_quant(model.params_b, &hw)
        .ok_or_else(|| "no quant fits this machine".to_string())?;
    let source = model.source.clone();
    let filename = mm::download::gguf_filename(&source, quant)
        .ok_or_else(|| "unresolvable model source".to_string())?;
    let url = mm::download::gguf_resolve_url(&source, quant)
        .ok_or_else(|| "unresolvable model url".to_string())?;
    let dest = mm::download::local_model_path(&source, quant)
        .ok_or_else(|| "model store dir unavailable".to_string())?;

    // Resolve the integrity sha256 from the HF file tree (egress, off the runtime).
    let sha = {
        let (source, filename) = (source.clone(), filename.clone());
        tokio::task::spawn_blocking(move || mm::hf::resolve_gguf_sha(&source, &filename))
            .await
            .map_err(|e| format!("resolve task: {e}"))?
            .map_err(|e| e.to_string())?
            .sha256
    };

    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create store dir: {e}"))?;
    }

    // Register a shared cancel flag so `ai_local_models_download_cancel` can abort
    // this transfer, then stream the download off the async runtime emitting progress.
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    cancels
        .0
        .lock()
        .expect("download-cancels lock")
        .insert(id.clone(), flag.clone());

    let progress_id = id.clone();
    let dl_flag = flag.clone();
    let result = tokio::task::spawn_blocking(move || {
        let on_progress = move |fetched: u64, total: u64| {
            let _ = app.emit(
                "ai:model-download-progress",
                ModelDownloadProgress {
                    id: progress_id.clone(),
                    bytes_fetched: fetched,
                    total_bytes: total,
                },
            );
        };
        let observer = mm::fetch::DownloadObserver {
            on_progress: &on_progress,
            cancel: &dl_flag,
        };
        mm::fetch::download_model(&url, &sha, &dest, Some(&observer))
    })
    .await;

    // Drop the flag entry whether the download finished, failed, or was cancelled.
    cancels
        .0
        .lock()
        .expect("download-cancels lock")
        .remove(&id);

    result
        .map_err(|e| format!("download task: {e}"))?
        .map_err(|e| e.to_string())
}

/// Per-download cancel flags keyed by model id, so [`ai_local_models_download_cancel`]
/// can abort an in-flight [`ai_local_models_download`]. The flag is inserted when a
/// download starts and removed when it ends; Tauri-managed shared state.
#[derive(Default)]
pub struct DownloadCancels(
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>>,
);

/// Cancel an in-flight model download (`ai_local_models_download_cancel`): flip the
/// shared flag its streaming observer checks before each read, so the transfer aborts
/// and the partial file is discarded. A no-op if no download for `id` is running.
#[tauri::command]
pub fn ai_local_models_download_cancel(cancels: tauri::State<'_, DownloadCancels>, id: String) {
    if let Some(flag) = cancels.0.lock().expect("download-cancels lock").get(&id) {
        flag.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}
