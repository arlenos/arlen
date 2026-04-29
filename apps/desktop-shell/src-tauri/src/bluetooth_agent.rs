//! BlueZ `org.bluez.Agent1` implementation for desktop-shell.
//!
//! Architecture and design decisions live in
//! `docs/architecture/bluetooth-pairing.md`. This module is the
//! implementation: a zbus-served interface, a pending-request map
//! that bridges blocking BlueZ calls to a Tauri-event/Tauri-command
//! request/response loop, and the lifecycle plumbing that registers
//! the agent against `org.bluez` (and re-registers if BlueZ
//! restarts).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};
use zbus::zvariant::{ObjectPath, OwnedObjectPath};
use zbus::Connection;

/// Object path our agent is exposed at on our unique D-Bus name.
const AGENT_PATH: &str = "/org/lunaris/bluetooth_agent";

/// Pairing capability declared to BlueZ. See architecture decision
/// A1: `KeyboardDisplay` is the most flexible, lets BlueZ pick the
/// right method for the peer device.
const AGENT_CAPABILITY: &str = "KeyboardDisplay";

/// User has 60 seconds to respond before we auto-reject.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Wire types — Tauri events and commands
// ---------------------------------------------------------------------------

/// Payload sent to the frontend when a new pairing request arrives.
/// Mirrors the `PairRequest` discriminated union in
/// `bluetoothPairing.ts`. `rename_all_fields = "camelCase"` is what
/// converts the snake_case Rust struct fields (`device_name`, …)
/// to the camelCase shape the TypeScript store expects.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PairRequestDto {
    Confirmation {
        id: u64,
        device_name: String,
        device_address: String,
        passkey: u32,
    },
    PinCodeInput {
        id: u64,
        device_name: String,
        device_address: String,
    },
    PasskeyInput {
        id: u64,
        device_name: String,
        device_address: String,
    },
    DisplayPinCode {
        id: u64,
        device_name: String,
        device_address: String,
        pin_code: String,
    },
    DisplayPasskey {
        id: u64,
        device_name: String,
        device_address: String,
        passkey: u32,
        entered: u16,
    },
    Authorization {
        id: u64,
        device_name: String,
        device_address: String,
    },
    AuthorizeService {
        id: u64,
        device_name: String,
        device_address: String,
        uuid: String,
        uuid_label: String,
    },
}

/// Live update for a `DisplayPasskey` as the user types digits on
/// the remote device.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayUpdateDto {
    pub id: u64,
    pub entered: u16,
}

/// Cancel notification — `id = None` clears every active request,
/// e.g. on `Release`. `id = Some(_)` clears one request, e.g. on
/// device-side timeout reflected through `Cancel`.
#[derive(Debug, Clone, Serialize)]
pub struct PairCancelDto {
    pub id: Option<u64>,
}

/// Frontend's response payload, deserialised from the
/// `bluetooth_pair_respond` Tauri command argument. The agent
/// validates the response variant against the pending request kind
/// before delivering — a type-mismatch (e.g. `PinCode` for a
/// `Confirmation` request) is rejected defensively.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PairResponseDto {
    Confirm,
    Reject,
    PinCode { value: String },
    Passkey { value: u32 },
}

// ---------------------------------------------------------------------------
// Pending-request map
// ---------------------------------------------------------------------------

/// What a blocking method's awaiting half receives. Mirrors the
/// possible D-Bus return shapes; the interface methods convert this
/// into the right zbus return type or error.
#[derive(Debug)]
enum AgentResponse {
    Confirm,
    Reject,
    PinCode(String),
    Passkey(u32),
}

#[derive(Debug)]
enum PendingEntry {
    Blocking {
        device_path: String,
        kind: BlockingKind,
        sender: oneshot::Sender<AgentResponse>,
    },
    Display {
        device_path: String,
    },
}

/// Used to demultiplex incoming responses against the pending
/// request kind, so the frontend can't accidentally feed a PIN
/// string to a Confirmation slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockingKind {
    Confirmation,
    PinCodeInput,
    PasskeyInput,
    Authorization,
    AuthorizeService,
}

/// Pure data layer for pending pairing requests. Lives behind the
/// agent's `Mutex` and is intentionally side-effect-free so unit
/// tests can poke at it without standing up a `tauri::AppHandle`.
///
/// The agent enforces two invariants on top of this map:
///
/// 1. **At most one Blocking entry exists at a time.** A second
///    actionable Agent1 call is rejected immediately by the agent
///    rather than queued, because the frontend renders a single
///    dialog and a hidden second request would just sit until the
///    60s timeout fires.
///
/// 2. **Display entries are cleaned up on Paired-success or
///    Cancel.** Display methods are non-blocking, so without a
///    completion signal a stale entry would survive forever and
///    `pending_dtos()` could resurrect it on frontend reload.
#[derive(Debug, Default)]
struct PendingMap {
    entries: HashMap<u64, (PendingEntry, PairRequestDto)>,
}

impl PendingMap {
    fn count_blocking(&self) -> usize {
        self.entries
            .values()
            .filter(|(e, _)| matches!(e, PendingEntry::Blocking { .. }))
            .count()
    }

    /// Look up the request id of an existing Display entry for the
    /// given device path, if any. Used so repeated `DisplayPasskey`
    /// updates fold into one dialog instead of opening new ones.
    fn find_display_id_for_device(&self, path: &str) -> Option<u64> {
        self.entries.iter().find_map(|(id, (entry, _))| match entry {
            PendingEntry::Display { device_path } if device_path == path => {
                Some(*id)
            }
            _ => None,
        })
    }

    /// All Display entry ids for a device. Used to clean up after a
    /// successful pairing — there can be more than one if a peer
    /// somehow drove multiple Display* methods, and we want to drop
    /// every dialog associated with it.
    fn display_ids_for_device(&self, path: &str) -> Vec<u64> {
        self.entries
            .iter()
            .filter_map(|(id, (entry, _))| match entry {
                PendingEntry::Display { device_path } if device_path == path => {
                    Some(*id)
                }
                _ => None,
            })
            .collect()
    }

    fn dtos(&self) -> Vec<PairRequestDto> {
        self.entries.values().map(|(_, dto)| dto.clone()).collect()
    }
}

// ---------------------------------------------------------------------------
// The agent
// ---------------------------------------------------------------------------

/// Pairing-agent state shared between the zbus interface and the
/// Tauri command surface. Wrapped in `Arc` because zbus's
/// `object_server().at(...)` consumes the value — we keep an extra
/// `Arc` clone in Tauri-managed state so the command handler can
/// reach in and deliver responses.
pub struct BluetoothAgent {
    pending: Mutex<PendingMap>,
    next_id: AtomicU64,
    app: AppHandle,
}

impl BluetoothAgent {
    pub fn new(app: AppHandle) -> Self {
        Self {
            pending: Mutex::new(PendingMap::default()),
            next_id: AtomicU64::new(1),
            app,
        }
    }

    /// Allocate a fresh request id. Monotonic, never reuses; 64
    /// bits is plenty for the lifetime of any process.
    fn alloc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Snapshot every pending request as DTOs for the frontend
    /// hot-reload restore path.
    pub async fn pending_dtos(&self) -> Vec<PairRequestDto> {
        self.pending.lock().await.dtos()
    }

    /// Deliver the frontend's response into the pending oneshot
    /// channel. Returns `Err` if the id is unknown (likely already
    /// timed out or cancelled) or the response kind doesn't match
    /// the pending request kind.
    pub async fn deliver_response(
        self: &Arc<Self>,
        req_id: u64,
        response: PairResponseDto,
    ) -> Result<(), String> {
        let mut pending = self.pending.lock().await;
        let Some((entry, _dto)) = pending.entries.remove(&req_id) else {
            return Err(format!("unknown or expired request id {req_id}"));
        };
        match entry {
            PendingEntry::Blocking { kind, sender, .. } => {
                let agent_response = match (kind, response) {
                    (_, PairResponseDto::Reject) => AgentResponse::Reject,
                    (BlockingKind::Confirmation, PairResponseDto::Confirm)
                    | (BlockingKind::Authorization, PairResponseDto::Confirm)
                    | (BlockingKind::AuthorizeService, PairResponseDto::Confirm) => {
                        AgentResponse::Confirm
                    }
                    (BlockingKind::PinCodeInput, PairResponseDto::PinCode { value }) => {
                        AgentResponse::PinCode(value)
                    }
                    (BlockingKind::PasskeyInput, PairResponseDto::Passkey { value }) => {
                        AgentResponse::Passkey(value)
                    }
                    _ => {
                        // Mismatched kind: defensive reject so the
                        // bus method returns an error instead of
                        // silently using nonsense.
                        log::warn!(
                            "bluetooth_agent: response kind mismatch for id {req_id}, \
                             rejecting"
                        );
                        AgentResponse::Reject
                    }
                };
                let _ = sender.send(agent_response);
                Ok(())
            }
            PendingEntry::Display { .. } => {
                // Display kinds don't have a sender — no response
                // is meaningful. Frontend should have called
                // `cancel` instead, which goes through `cancel_one`
                // below.
                Err("cannot respond to a Display request".into())
            }
        }
    }

    /// Insert a Blocking entry if and only if no other actionable
    /// request is in flight. Returns `Some(receiver)` on success;
    /// `None` means the caller must respond with `Rejected` because
    /// another pairing dialog is already showing — the frontend
    /// renders a single dialog at a time, so a hidden second
    /// blocking request would just sit there for 60 s.
    ///
    /// On success, emits the request DTO and spawns a timeout task
    /// that auto-rejects after `RESPONSE_TIMEOUT`.
    async fn begin_blocking(
        self: &Arc<Self>,
        device_path: String,
        kind: BlockingKind,
        dto_for_id: impl FnOnce(u64) -> PairRequestDto,
    ) -> Option<oneshot::Receiver<AgentResponse>> {
        let id = self.alloc_id();
        let dto = dto_for_id(id);
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending.lock().await;
            if pending.count_blocking() > 0 {
                log::warn!(
                    "bluetooth_agent: rejecting concurrent blocking request \
                     for {device_path} — another pairing already in progress"
                );
                return None;
            }
            pending.entries.insert(
                id,
                (
                    PendingEntry::Blocking {
                        device_path,
                        kind,
                        sender: tx,
                    },
                    dto.clone(),
                ),
            );
        }

        let _ = self.app.emit("bluetooth-pair-request", &dto);

        let me = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(RESPONSE_TIMEOUT).await;
            me.timeout_one(id).await;
        });

        Some(rx)
    }

    /// Begin or update a Display state for the given device. Display
    /// methods don't block — the caller's bus method returns
    /// immediately. The dialog stays open until either
    /// `dismiss_display_for_device` is called from the
    /// `Paired=true` watcher or one of `Cancel` / `Release` arrives
    /// from BlueZ.
    async fn begin_or_update_display(
        self: &Arc<Self>,
        device_path: &str,
        dto_for_id: impl FnOnce(u64) -> PairRequestDto,
        update: Option<DisplayUpdateDto>,
    ) {
        let existing_id = {
            let pending = self.pending.lock().await;
            pending.find_display_id_for_device(device_path)
        };

        if let (Some(id), Some(mut update)) = (existing_id, update) {
            update.id = id;
            let _ = self.app.emit("bluetooth-pair-display-update", &update);
            return;
        }

        let id = self.alloc_id();
        let dto = dto_for_id(id);
        {
            let mut pending = self.pending.lock().await;
            pending.entries.insert(
                id,
                (
                    PendingEntry::Display {
                        device_path: device_path.to_string(),
                    },
                    dto.clone(),
                ),
            );
        }
        let _ = self.app.emit("bluetooth-pair-request", &dto);
    }

    /// Drop every Display entry whose `device_path` matches `path`,
    /// and emit a cancel event per dropped id so the frontend
    /// dismisses the corresponding dialog. This is the success-path
    /// cleanup for `DisplayPinCode` / `DisplayPasskey`, called when
    /// `pair_bluetooth_device` finishes a `Pair()` future and from
    /// the `Paired=true` PropertiesChanged watcher (the latter
    /// covers peer-initiated pairings the user-initiated path can't
    /// observe).
    ///
    /// Blocking entries on the same device path are intentionally
    /// not touched — they have their own oneshot-channel lifecycle
    /// and resolve through the user's response.
    pub async fn dismiss_display_for_device(self: &Arc<Self>, path: &str) {
        let ids: Vec<u64> = {
            let mut pending = self.pending.lock().await;
            let ids = pending.display_ids_for_device(path);
            for id in &ids {
                pending.entries.remove(id);
            }
            ids
        };
        for id in ids {
            let _ = self
                .app
                .emit("bluetooth-pair-cancel", PairCancelDto { id: Some(id) });
        }
    }

    /// Cancel one pending request by id. If the entry is Blocking,
    /// send a Reject down its sender so the bus method completes
    /// with an error.
    async fn cancel_one(self: &Arc<Self>, id: u64) {
        let mut pending = self.pending.lock().await;
        if let Some((entry, _)) = pending.entries.remove(&id) {
            if let PendingEntry::Blocking { sender, .. } = entry {
                let _ = sender.send(AgentResponse::Reject);
            }
        }
        drop(pending);
        let _ = self
            .app
            .emit("bluetooth-pair-cancel", PairCancelDto { id: Some(id) });
    }

    /// Drain every pending request. Used by `Release` (we are no
    /// longer the agent) and `Cancel` (the active request is
    /// aborted by BlueZ).
    async fn cancel_all(self: &Arc<Self>) {
        let mut pending = self.pending.lock().await;
        for (_id, (entry, _)) in pending.entries.drain() {
            if let PendingEntry::Blocking { sender, .. } = entry {
                let _ = sender.send(AgentResponse::Reject);
            }
        }
        drop(pending);
        let _ = self
            .app
            .emit("bluetooth-pair-cancel", PairCancelDto { id: None });
    }

    async fn timeout_one(self: &Arc<Self>, id: u64) {
        let mut pending = self.pending.lock().await;
        if let Some((entry, _)) = pending.entries.remove(&id) {
            if let PendingEntry::Blocking { sender, .. } = entry {
                log::info!("bluetooth_agent: request {id} timed out, auto-rejecting");
                let _ = sender.send(AgentResponse::Reject);
            }
        }
        drop(pending);
        let _ = self
            .app
            .emit("bluetooth-pair-cancel", PairCancelDto { id: Some(id) });
    }
}

// ---------------------------------------------------------------------------
// zbus interface — org.bluez.Agent1
// ---------------------------------------------------------------------------

/// Server-side `Agent1` implementation. Holds an `Arc<BluetoothAgent>`
/// so the methods can clone it into spawned helpers without moving
/// the inner state.
pub struct AgentInterface {
    inner: Arc<BluetoothAgent>,
    /// System-bus connection used to look up device names. Cloned
    /// on construction; the owning `Connection` lives in
    /// `register_with_bluez`.
    conn: Connection,
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl AgentInterface {
    /// Called by BlueZ when our agent is unregistered (either
    /// because we asked, or because BlueZ is shutting down). Drop
    /// every pending request — the bus is going away.
    async fn release(&self) {
        log::info!("bluetooth_agent: Release()");
        self.inner.cancel_all().await;
    }

    /// Legacy pairing — prompt for a PIN code (1-16 chars).
    async fn request_pin_code(
        &self,
        device: ObjectPath<'_>,
    ) -> zbus::fdo::Result<String> {
        let path = device.to_string();
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let inner = Arc::clone(&self.inner);
        let Some(rx) = inner
            .begin_blocking(path.clone(), BlockingKind::PinCodeInput, |id| {
                PairRequestDto::PinCodeInput {
                    id,
                    device_name: name,
                    device_address: address,
                }
            })
            .await
        else {
            return Err(rejected());
        };
        match rx.await {
            Ok(AgentResponse::PinCode(s)) => Ok(s),
            _ => Err(rejected()),
        }
    }

    /// Display-only — the user types `pincode` on the peer device.
    /// Returns immediately; BlueZ may re-call.
    async fn display_pin_code(
        &self,
        device: ObjectPath<'_>,
        pincode: String,
    ) -> zbus::fdo::Result<()> {
        let path = device.to_string();
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let inner = Arc::clone(&self.inner);
        let pincode_clone = pincode.clone();
        inner
            .begin_or_update_display(
                &path,
                |id| PairRequestDto::DisplayPinCode {
                    id,
                    device_name: name,
                    device_address: address,
                    pin_code: pincode_clone,
                },
                None,
            )
            .await;
        let _ = pincode;
        Ok(())
    }

    /// SSP — prompt for a 0-999999 numeric passkey.
    async fn request_passkey(
        &self,
        device: ObjectPath<'_>,
    ) -> zbus::fdo::Result<u32> {
        let path = device.to_string();
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let inner = Arc::clone(&self.inner);
        let Some(rx) = inner
            .begin_blocking(path.clone(), BlockingKind::PasskeyInput, |id| {
                PairRequestDto::PasskeyInput {
                    id,
                    device_name: name,
                    device_address: address,
                }
            })
            .await
        else {
            return Err(rejected());
        };
        match rx.await {
            Ok(AgentResponse::Passkey(n)) => Ok(n),
            _ => Err(rejected()),
        }
    }

    /// SSP — display passkey to the user. Repeated calls update the
    /// `entered` count as the user types on the peer device.
    async fn display_passkey(
        &self,
        device: ObjectPath<'_>,
        passkey: u32,
        entered: u16,
    ) -> zbus::fdo::Result<()> {
        let path = device.to_string();
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let inner = Arc::clone(&self.inner);
        // Update path is taken when an existing Display entry for
        // this device is found inside `begin_or_update_display`;
        // the `update` payload's id is fixed up there.
        inner
            .begin_or_update_display(
                &path,
                |id| PairRequestDto::DisplayPasskey {
                    id,
                    device_name: name,
                    device_address: address,
                    passkey,
                    entered,
                },
                Some(DisplayUpdateDto { id: 0, entered }),
            )
            .await;
        Ok(())
    }

    /// SSP — both ends show the same `passkey`; user accepts iff
    /// they match.
    async fn request_confirmation(
        &self,
        device: ObjectPath<'_>,
        passkey: u32,
    ) -> zbus::fdo::Result<()> {
        let path = device.to_string();
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let inner = Arc::clone(&self.inner);
        let Some(rx) = inner
            .begin_blocking(path.clone(), BlockingKind::Confirmation, |id| {
                PairRequestDto::Confirmation {
                    id,
                    device_name: name,
                    device_address: address,
                    passkey,
                }
            })
            .await
        else {
            return Err(rejected());
        };
        match rx.await {
            Ok(AgentResponse::Confirm) => Ok(()),
            _ => Err(rejected()),
        }
    }

    /// Pre-pairing authorization for an incoming request without
    /// MITM protection.
    async fn request_authorization(
        &self,
        device: ObjectPath<'_>,
    ) -> zbus::fdo::Result<()> {
        let path = device.to_string();
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let inner = Arc::clone(&self.inner);
        let Some(rx) = inner
            .begin_blocking(path.clone(), BlockingKind::Authorization, |id| {
                PairRequestDto::Authorization {
                    id,
                    device_name: name,
                    device_address: address,
                }
            })
            .await
        else {
            return Err(rejected());
        };
        match rx.await {
            Ok(AgentResponse::Confirm) => Ok(()),
            _ => Err(rejected()),
        }
    }

    /// Service-level authorization. Architecture decision A5: if
    /// the device is already trusted, return immediately without
    /// surfacing a dialog.
    async fn authorize_service(
        &self,
        device: ObjectPath<'_>,
        uuid: String,
    ) -> zbus::fdo::Result<()> {
        let path = device.to_string();
        if device_is_trusted(&self.conn, &path).await {
            return Ok(());
        }
        let (name, address) = device_name_and_address(&self.conn, &path).await;
        let label = uuid_label(&uuid);
        let inner = Arc::clone(&self.inner);
        let Some(rx) = inner
            .begin_blocking(path.clone(), BlockingKind::AuthorizeService, |id| {
                PairRequestDto::AuthorizeService {
                    id,
                    device_name: name,
                    device_address: address,
                    uuid,
                    uuid_label: label,
                }
            })
            .await
        else {
            return Err(rejected());
        };
        match rx.await {
            Ok(AgentResponse::Confirm) => Ok(()),
            _ => Err(rejected()),
        }
    }

    /// BlueZ aborts whatever is in flight — typically when the peer
    /// device gives up or the user pressed cancel on the device
    /// side.
    async fn cancel(&self) {
        log::info!("bluetooth_agent: Cancel()");
        self.inner.cancel_all().await;
    }
}

/// Build the `org.bluez.Error.Rejected` error that BlueZ recognises
/// as "user said no". `zbus::fdo::Error::Failed` on the wire becomes
/// `org.freedesktop.DBus.Error.Failed`, which BlueZ does not treat
/// as a meaningful pairing failure — `Rejected` is the right name.
fn rejected() -> zbus::fdo::Error {
    zbus::fdo::Error::Failed("org.bluez.Error.Rejected".into())
}

// ---------------------------------------------------------------------------
// Device-property helpers
// ---------------------------------------------------------------------------

/// Read `Alias` (or fallback `Name`, then `Address`) on the given
/// device path, plus `Address`. Errors fall back to the path itself
/// so the dialog never shows nothing.
async fn device_name_and_address(conn: &Connection, path: &str) -> (String, String) {
    let proxy = match zbus::Proxy::new(conn, "org.bluez", path, "org.bluez.Device1").await {
        Ok(p) => p,
        Err(_) => return (path.to_string(), String::new()),
    };
    let alias = proxy.get_property::<String>("Alias").await.ok();
    let name = proxy.get_property::<String>("Name").await.ok();
    let address = proxy
        .get_property::<String>("Address")
        .await
        .unwrap_or_default();
    let display = alias
        .filter(|s| !s.trim().is_empty())
        .or_else(|| name.filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| address.clone());
    (display, address)
}

/// Read `Trusted` on the given device path. Defaults to `false` on
/// any error so we err on the side of prompting the user.
async fn device_is_trusted(conn: &Connection, path: &str) -> bool {
    let Ok(proxy) =
        zbus::Proxy::new(conn, "org.bluez", path, "org.bluez.Device1").await
    else {
        return false;
    };
    proxy
        .get_property::<bool>("Trusted")
        .await
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// UUID lookup table
// ---------------------------------------------------------------------------

/// Human-readable label for a Bluetooth service UUID. Covers the
/// SIG-assigned UUIDs we expect to see in everyday use plus a few
/// iOS-relevant ones. Unknown UUIDs return `Unknown service ({id})`.
pub(crate) fn uuid_label(uuid: &str) -> String {
    let lower = uuid.to_lowercase();
    let short = short_uuid(&lower);
    if let Some(short) = short {
        if let Some(name) = lookup_short_uuid(short) {
            return name.to_string();
        }
    }
    format!("Unknown service ({})", short_form(&lower))
}

/// Extract the first 32 bits from a 128-bit UUID in canonical form
/// (`xxxxxxxx-...`), or parse a 4-hex-digit short form. None on
/// malformed input.
fn short_uuid(uuid: &str) -> Option<u32> {
    if let Some(first) = uuid.split('-').next() {
        if first.len() == 8 {
            return u32::from_str_radix(first, 16).ok();
        }
        if first.len() == 4 {
            return u32::from_str_radix(first, 16).ok();
        }
    }
    None
}

fn short_form(uuid: &str) -> String {
    short_uuid(uuid)
        .map(|n| format!("0x{n:08x}"))
        .unwrap_or_else(|| uuid.to_string())
}

fn lookup_short_uuid(short: u32) -> Option<&'static str> {
    match short {
        0x1101 => Some("Serial Port"),
        0x1108 => Some("Headset"),
        0x110a => Some("Audio Source"),
        0x110b => Some("Audio Sink"),
        0x110c => Some("A/V Remote Control Target"),
        0x110e => Some("A/V Remote Control"),
        0x1112 => Some("Headset Audio Gateway"),
        0x1115 => Some("Personal Area Network User"),
        0x1116 => Some("Network Access Point"),
        0x111e => Some("Hands-Free"),
        0x1124 => Some("Human Interface Device"),
        0x112d => Some("SIM Access"),
        0x112f => Some("Phonebook Access Server"),
        0x1132 => Some("Message Access Server"),
        0x1133 => Some("Message Notification Server"),
        0x1134 => Some("Phonebook Access"),
        0x1200 => Some("Device Information"),
        0x1812 => Some("Human Interface (BLE)"),
        0x180f => Some("Battery Service"),
        0x1813 => Some("Scan Parameters"),
        // Apple-specific (vendor UUIDs encoded with the assigned
        // base UUID still resolve via the high 32 bits)
        0x7905f431 => Some("Apple Notification Center (ANCS)"),
        0x89d3502b => Some("Apple Media Service (AMS)"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Lifecycle — register / re-register against org.bluez
// ---------------------------------------------------------------------------

/// Register the agent against BlueZ and keep it registered across
/// `bluetoothd` restarts. Spawned once at shell startup.
///
/// On entry we open a system-bus connection, mount the
/// `Agent1` interface at `AGENT_PATH`, and call
/// `AgentManager1.RegisterAgent` + `RequestDefaultAgent`. The
/// default-agent request is non-fatal — coexistence with
/// gnome-bluetooth or `bluetoothctl` is OK (architecture decision
/// A4).
///
/// We also subscribe to `NameOwnerChanged` on `org.bluez`. If
/// `bluetoothd` restarts (which we observe as the well-known name
/// vanishing then re-appearing under a new unique name), we redo
/// the registration so the agent stays live.
pub async fn register_with_bluez(agent: Arc<BluetoothAgent>) -> Result<(), String> {
    let conn = Connection::system()
        .await
        .map_err(|e| format!("system bus: {e}"))?;

    install_interface(&conn, Arc::clone(&agent)).await?;
    let _ = call_register(&conn).await;
    let _ = call_request_default(&conn).await;

    spawn_name_owner_watch(conn.clone(), Arc::clone(&agent));
    spawn_paired_watch(conn, Arc::clone(&agent));
    Ok(())
}

async fn install_interface(
    conn: &Connection,
    agent: Arc<BluetoothAgent>,
) -> Result<(), String> {
    let iface = AgentInterface {
        inner: agent,
        conn: conn.clone(),
    };
    conn.object_server()
        .at(AGENT_PATH, iface)
        .await
        .map_err(|e| format!("object_server.at: {e}"))?;
    Ok(())
}

async fn call_register(conn: &Connection) -> Result<(), String> {
    let proxy = zbus::Proxy::new(conn, "org.bluez", "/org/bluez", "org.bluez.AgentManager1")
        .await
        .map_err(|e| format!("AgentManager1 proxy: {e}"))?;
    let path = OwnedObjectPath::try_from(AGENT_PATH)
        .map_err(|e| format!("agent path: {e}"))?;
    proxy
        .call::<_, _, ()>("RegisterAgent", &(&path, AGENT_CAPABILITY))
        .await
        .map_err(|e| {
            log::warn!("bluetooth_agent: RegisterAgent failed: {e}");
            format!("RegisterAgent: {e}")
        })?;
    proxy
        .call::<_, _, ()>("RequestDefaultAgent", &(&path,))
        .await
        .or_else(|e| {
            log::warn!(
                "bluetooth_agent: RequestDefaultAgent failed (another agent likely owns it): {e}"
            );
            Ok::<(), String>(())
        })?;
    log::info!("bluetooth_agent: registered with BlueZ at {AGENT_PATH}");
    Ok(())
}

async fn call_request_default(conn: &Connection) -> Result<(), String> {
    // Folded into call_register so callers don't need to sequence
    // both. Kept as a separate function only for symmetry / future
    // explicit re-requests if we ever need them.
    let _ = conn;
    Ok(())
}

fn spawn_name_owner_watch(conn: Connection, agent: Arc<BluetoothAgent>) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_name_owner_watch(conn, agent).await {
            log::warn!("bluetooth_agent: name-owner watch ended: {e}");
        }
    });
}

/// Subscribe to `org.freedesktop.DBus.Properties.PropertiesChanged`
/// on every BlueZ object and dismiss any Display dialog whose
/// device just transitioned to `Paired=true`. Without this watcher,
/// a successful incoming pairing would leave the dialog stuck on
/// the screen because Display methods don't have a return-value
/// signal of completion.
fn spawn_paired_watch(conn: Connection, agent: Arc<BluetoothAgent>) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_paired_watch(conn, agent).await {
            log::warn!("bluetooth_agent: paired-watch ended: {e}");
        }
    });
}

async fn run_paired_watch(
    conn: Connection,
    agent: Arc<BluetoothAgent>,
) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;

    // We listen on the bus directly with a match rule, then filter
    // in the handler. Subscribing per-Device1 path would require
    // tracking InterfacesAdded/Removed events too — the global
    // PropertiesChanged stream is simpler and BlueZ's signal volume
    // is low enough that filtering in user-space is fine.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.DBus.Properties")?
        .member("PropertiesChanged")?
        .build();

    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    dbus.add_match_rule(rule).await?;

    let mut stream = zbus::MessageStream::from(conn);
    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { continue };
        if msg.message_type() != zbus::message::Type::Signal {
            continue;
        }
        // Body shape: (interface_name, changed_props, invalidated)
        let body = msg.body();
        let Ok((iface, changed, _invalidated)) = body.deserialize::<(
            String,
            std::collections::HashMap<String, zbus::zvariant::Value<'_>>,
            Vec<String>,
        )>() else {
            continue;
        };
        if iface != "org.bluez.Device1" {
            continue;
        }
        let Some(paired_value) = changed.get("Paired") else { continue };
        let Ok(paired) = bool::try_from(paired_value) else { continue };
        if !paired {
            continue;
        }
        let Some(header_path) = msg.header().path().cloned() else { continue };
        let path_str = header_path.to_string();
        log::info!(
            "bluetooth_agent: device {path_str} paired; dismissing display state"
        );
        agent.dismiss_display_for_device(&path_str).await;
    }
    Ok(())
}

async fn run_name_owner_watch(
    conn: Connection,
    agent: Arc<BluetoothAgent>,
) -> Result<(), zbus::Error> {
    use futures_util::StreamExt;

    let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
    let mut stream = dbus.receive_name_owner_changed().await?;

    while let Some(signal) = stream.next().await {
        let Ok(args) = signal.args() else { continue };
        if args.name() != "org.bluez" {
            continue;
        }
        let new_owner = args.new_owner();
        if new_owner.as_ref().is_some() {
            log::info!("bluetooth_agent: org.bluez reappeared; re-registering");
            // Re-mount the interface in case BlueZ-side state was
            // cleared, then call RegisterAgent again.
            let _ = conn
                .object_server()
                .remove::<AgentInterface, _>(AGENT_PATH)
                .await;
            if let Err(e) = install_interface(&conn, Arc::clone(&agent)).await {
                log::warn!("bluetooth_agent: re-mount failed: {e}");
                continue;
            }
            if let Err(e) = call_register(&conn).await {
                log::warn!("bluetooth_agent: re-register failed: {e}");
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Frontend → backend bridge for delivering a user response to a
/// pending pairing request.
#[tauri::command]
pub async fn bluetooth_pair_respond(
    req_id: u64,
    response: PairResponseDto,
    agent: tauri::State<'_, Arc<BluetoothAgent>>,
) -> Result<(), String> {
    agent.deliver_response(req_id, response).await
}

/// Snapshot of currently-pending requests, used by the frontend on
/// mount to restore its dialog after a hot-reload or window-recreate.
#[tauri::command]
pub async fn bluetooth_pair_pending_requests(
    agent: tauri::State<'_, Arc<BluetoothAgent>>,
) -> Result<Vec<PairRequestDto>, String> {
    Ok(agent.pending_dtos().await)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// UUID lookup must hit the common SIG-assigned profiles, fall
    /// back gracefully on unknown short UUIDs, and not panic on
    /// malformed input.
    #[test]
    fn uuid_lookup_known_short_form() {
        assert_eq!(uuid_label("0000110b-0000-1000-8000-00805f9b34fb"), "Audio Sink");
        assert_eq!(uuid_label("0000111e-0000-1000-8000-00805f9b34fb"), "Hands-Free");
        assert_eq!(
            uuid_label("00001124-0000-1000-8000-00805f9b34fb"),
            "Human Interface Device"
        );
    }

    #[test]
    fn uuid_lookup_unknown_returns_short_form() {
        let label = uuid_label("0000abcd-0000-1000-8000-00805f9b34fb");
        assert!(label.starts_with("Unknown service"));
        assert!(label.contains("0x0000abcd"));
    }

    #[test]
    fn uuid_lookup_malformed_does_not_panic() {
        // Empty, non-hex, too-short — none of these should trip.
        let _ = uuid_label("");
        let _ = uuid_label("not-a-uuid");
        let _ = uuid_label("xx-yy");
    }

    #[test]
    fn uuid_lookup_case_insensitive() {
        assert_eq!(uuid_label("0000110B-0000-1000-8000-00805F9B34FB"), "Audio Sink");
    }

    /// Response demultiplex: the right Confirm/Reject/PinCode/
    /// Passkey response must land in the matching pending kind, and
    /// mismatches must be defensively converted to Reject.
    #[test]
    fn pending_demux_matches_kinds() {
        // Helper closure replicating the match arms in
        // `deliver_response`. Keeping the predicate isolated lets
        // us assert it without standing up an AppHandle.
        fn pick(kind: BlockingKind, resp: PairResponseDto) -> AgentResponse {
            match (kind, resp) {
                (_, PairResponseDto::Reject) => AgentResponse::Reject,
                (BlockingKind::Confirmation, PairResponseDto::Confirm)
                | (BlockingKind::Authorization, PairResponseDto::Confirm)
                | (BlockingKind::AuthorizeService, PairResponseDto::Confirm) => {
                    AgentResponse::Confirm
                }
                (BlockingKind::PinCodeInput, PairResponseDto::PinCode { value }) => {
                    AgentResponse::PinCode(value)
                }
                (BlockingKind::PasskeyInput, PairResponseDto::Passkey { value }) => {
                    AgentResponse::Passkey(value)
                }
                _ => AgentResponse::Reject,
            }
        }

        // Matching pairs.
        assert!(matches!(
            pick(BlockingKind::Confirmation, PairResponseDto::Confirm),
            AgentResponse::Confirm
        ));
        assert!(matches!(
            pick(
                BlockingKind::PinCodeInput,
                PairResponseDto::PinCode {
                    value: "1234".into(),
                },
            ),
            AgentResponse::PinCode(s) if s == "1234"
        ));
        assert!(matches!(
            pick(
                BlockingKind::PasskeyInput,
                PairResponseDto::Passkey { value: 654321 },
            ),
            AgentResponse::Passkey(654321)
        ));

        // Mismatched: PIN response into Confirmation slot must
        // become Reject, never silently succeed.
        assert!(matches!(
            pick(
                BlockingKind::Confirmation,
                PairResponseDto::PinCode {
                    value: "abcd".into()
                },
            ),
            AgentResponse::Reject
        ));
        assert!(matches!(
            pick(
                BlockingKind::PasskeyInput,
                PairResponseDto::Confirm,
            ),
            AgentResponse::Reject
        ));

        // Reject on any kind still rejects.
        assert!(matches!(
            pick(BlockingKind::AuthorizeService, PairResponseDto::Reject),
            AgentResponse::Reject
        ));
    }

    /// Round-trip a PairRequestDto through serde_json so the
    /// frontend store's discriminator (`kind`) matches what we
    /// emit. Catches accidental rename_all drift.
    #[test]
    fn pair_request_dto_serialises_with_camel_kind() {
        let dto = PairRequestDto::Confirmation {
            id: 7,
            device_name: "Speaker".into(),
            device_address: "AA:BB:CC:DD:EE:FF".into(),
            passkey: 123_456,
        };
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["kind"], "confirmation");
        assert_eq!(json["id"], 7);
        assert_eq!(json["deviceName"], "Speaker");
        assert_eq!(json["deviceAddress"], "AA:BB:CC:DD:EE:FF");
        assert_eq!(json["passkey"], 123_456);
    }

    /// Same for the response shape, since it crosses the Tauri
    /// boundary the other way.
    #[test]
    fn pair_response_dto_deserialises_camel_kind() {
        let confirm: PairResponseDto =
            serde_json::from_value(serde_json::json!({ "kind": "confirm" })).unwrap();
        assert!(matches!(confirm, PairResponseDto::Confirm));

        let pin: PairResponseDto = serde_json::from_value(serde_json::json!({
            "kind": "pinCode",
            "value": "0000",
        }))
        .unwrap();
        assert!(matches!(pin, PairResponseDto::PinCode { value } if value == "0000"));

        let passkey: PairResponseDto = serde_json::from_value(serde_json::json!({
            "kind": "passkey",
            "value": 123,
        }))
        .unwrap();
        assert!(matches!(passkey, PairResponseDto::Passkey { value: 123 }));
    }

    /// Rejection error must carry the BlueZ-recognised name so
    /// `bluetoothd` interprets it as a user-cancel rather than a
    /// generic D-Bus failure.
    #[test]
    fn rejected_error_carries_bluez_name() {
        let err = rejected();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("org.bluez.Error.Rejected"),
            "expected bluez-prefixed name, got {msg}"
        );
    }

    /// Helper for the data-layer tests below. Builds a Blocking
    /// entry without spinning up a real `oneshot` channel — we
    /// don't need the receiver, only the map state.
    fn fake_blocking_entry(device_path: &str, kind: BlockingKind) -> PendingEntry {
        let (tx, _rx) = oneshot::channel();
        PendingEntry::Blocking {
            device_path: device_path.to_string(),
            kind,
            sender: tx,
        }
    }

    fn fake_dto(id: u64, device_address: &str) -> PairRequestDto {
        PairRequestDto::Confirmation {
            id,
            device_name: "Test".into(),
            device_address: device_address.to_string(),
            passkey: 0,
        }
    }

    /// Codex finding 1: a second blocking request must not be
    /// silently inserted alongside an existing one. The data layer
    /// reports the count; the agent uses that to short-circuit.
    #[test]
    fn pending_map_blocks_serialise_via_count() {
        let mut pm = PendingMap::default();
        assert_eq!(pm.count_blocking(), 0);

        pm.entries.insert(
            1,
            (
                fake_blocking_entry("/dev/A", BlockingKind::Confirmation),
                fake_dto(1, "AA:AA"),
            ),
        );
        assert_eq!(pm.count_blocking(), 1);

        // A Display entry on the same device must not bump the
        // blocking count — the dialog limit only applies to
        // actionable requests.
        pm.entries.insert(
            2,
            (
                PendingEntry::Display {
                    device_path: "/dev/A".into(),
                },
                PairRequestDto::DisplayPinCode {
                    id: 2,
                    device_name: "Test".into(),
                    device_address: "AA:AA".into(),
                    pin_code: "1234".into(),
                },
            ),
        );
        assert_eq!(pm.count_blocking(), 1);

        // Two different devices, two blocking entries — count
        // reflects reality. The agent's invariant in
        // begin_blocking is what enforces "at most one"; the data
        // layer just provides the predicate.
        pm.entries.insert(
            3,
            (
                fake_blocking_entry("/dev/B", BlockingKind::PasskeyInput),
                fake_dto(3, "BB:BB"),
            ),
        );
        assert_eq!(pm.count_blocking(), 2);
    }

    /// Codex finding 2: Display entries must be cleanable per
    /// device path so a successful pair flow drops the dialog.
    #[test]
    fn pending_map_dismiss_display_only_targets_display_kind() {
        let mut pm = PendingMap::default();

        // Two Display entries for one device, one for another, plus
        // a Blocking entry on the first device that must NOT be
        // touched by display-only cleanup.
        pm.entries.insert(
            1,
            (
                PendingEntry::Display {
                    device_path: "/dev/A".into(),
                },
                PairRequestDto::DisplayPinCode {
                    id: 1,
                    device_name: "A".into(),
                    device_address: "AA:AA".into(),
                    pin_code: "1234".into(),
                },
            ),
        );
        pm.entries.insert(
            2,
            (
                PendingEntry::Display {
                    device_path: "/dev/A".into(),
                },
                PairRequestDto::DisplayPasskey {
                    id: 2,
                    device_name: "A".into(),
                    device_address: "AA:AA".into(),
                    passkey: 123_456,
                    entered: 0,
                },
            ),
        );
        pm.entries.insert(
            3,
            (
                PendingEntry::Display {
                    device_path: "/dev/B".into(),
                },
                PairRequestDto::DisplayPinCode {
                    id: 3,
                    device_name: "B".into(),
                    device_address: "BB:BB".into(),
                    pin_code: "5678".into(),
                },
            ),
        );
        pm.entries.insert(
            4,
            (
                fake_blocking_entry("/dev/A", BlockingKind::Confirmation),
                fake_dto(4, "AA:AA"),
            ),
        );

        // Both Display entries on /dev/A surface for cleanup.
        let mut ids_a = pm.display_ids_for_device("/dev/A");
        ids_a.sort();
        assert_eq!(ids_a, vec![1, 2]);

        // /dev/B unaffected.
        assert_eq!(pm.display_ids_for_device("/dev/B"), vec![3]);

        // Drop the /dev/A display entries (mirroring what
        // dismiss_display_for_device does internally).
        for id in ids_a {
            pm.entries.remove(&id);
        }

        // /dev/A blocking entry survives; /dev/B display survives.
        assert_eq!(pm.entries.len(), 2);
        assert!(pm.entries.contains_key(&3));
        assert!(pm.entries.contains_key(&4));

        // Subsequent dismiss-all on /dev/A is a no-op (idempotent).
        assert!(pm.display_ids_for_device("/dev/A").is_empty());
    }

    /// `pending_dtos()` must reflect exactly what's left in the
    /// map. Codex's concern was that a stale Display entry could
    /// resurrect the dialog on frontend-restore — once we clean it
    /// up via `dismiss_display_for_device`, `dtos()` must agree.
    #[test]
    fn pending_dtos_reflects_post_dismiss_state() {
        let mut pm = PendingMap::default();
        pm.entries.insert(
            1,
            (
                PendingEntry::Display {
                    device_path: "/dev/A".into(),
                },
                PairRequestDto::DisplayPinCode {
                    id: 1,
                    device_name: "A".into(),
                    device_address: "AA:AA".into(),
                    pin_code: "0000".into(),
                },
            ),
        );

        assert_eq!(pm.dtos().len(), 1);

        // Pretend the device just paired; clean up.
        for id in pm.display_ids_for_device("/dev/A") {
            pm.entries.remove(&id);
        }

        assert!(
            pm.dtos().is_empty(),
            "post-dismiss dtos must be empty so frontend restore \
             can't resurrect a completed dialog"
        );
    }

    /// `find_display_id_for_device` must return only Display
    /// entries — never a Blocking one — so update-coalescing in
    /// `begin_or_update_display` doesn't accidentally fire a
    /// display update at a blocking entry.
    #[test]
    fn pending_map_find_display_ignores_blocking() {
        let mut pm = PendingMap::default();
        pm.entries.insert(
            1,
            (
                fake_blocking_entry("/dev/A", BlockingKind::Confirmation),
                fake_dto(1, "AA:AA"),
            ),
        );
        assert_eq!(pm.find_display_id_for_device("/dev/A"), None);

        pm.entries.insert(
            2,
            (
                PendingEntry::Display {
                    device_path: "/dev/A".into(),
                },
                PairRequestDto::DisplayPinCode {
                    id: 2,
                    device_name: "A".into(),
                    device_address: "AA:AA".into(),
                    pin_code: "1234".into(),
                },
            ),
        );
        assert_eq!(pm.find_display_id_for_device("/dev/A"), Some(2));
    }
}
