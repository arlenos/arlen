//! The pure journal-line classifier: a `journalctl --output=json` line in, an
//! optional coarse [`ServiceEvent`] out.
//!
//! This is the heart of the Tier-2 ingestion (the gap-audit's "journald
//! per-service parser"): the systemd journal is the source, but only a small,
//! deliberately non-sensitive set of service transitions is lifted into an
//! event. The classifier is a pure function over a parsed line so it is fully
//! unit-testable without a running journal, and so the privacy posture is
//! provable by reading one file: it can only ever emit the fields it constructs
//! here, and it never copies an SSID, a credential, or free-form log text.
//!
//! Three services are recognised, matching the gap-audit's named set:
//! NetworkManager (device connectivity up/down, the interface name only, never
//! the SSID), bluetoothd (a device connect/disconnect, no device address), and
//! systemd-logind (a login session opening/closing, the numeric session id).
//! Anything else, and any unrecognised message within those services, classifies
//! to `None` (the conservative default: a line we do not understand is dropped,
//! never emitted as a guess).

/// A single journal entry reduced to the two fields the classifier reads. The
/// daemon builds one from a `journalctl --output=json` line; the unit and
/// identifier select the service, the message carries the transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalLine {
    /// `_SYSTEMD_UNIT`, e.g. `NetworkManager.service` (empty if absent).
    pub unit: String,
    /// `SYSLOG_IDENTIFIER`, e.g. `NetworkManager` (the fallback selector).
    pub identifier: String,
    /// `MESSAGE`, the human-readable log text.
    pub message: String,
}

/// A coarse, non-sensitive service transition - the only thing this tier emits.
///
/// Mirrors the `ServiceEventPayload` proto: `service` is the normalized origin,
/// `kind` the transition, `detail` a coarse non-sensitive identifier (an
/// interface name or a session id; never an SSID, an address or a secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEvent {
    /// `"network"`, `"bluetooth"` or `"session"`.
    pub service: String,
    /// The transition, e.g. `"device-up"`, `"connected"`, `"session-opened"`.
    pub kind: String,
    /// A coarse non-sensitive id (interface name, session id); may be empty.
    pub detail: String,
}

/// Parse one `journalctl --output=json` line into a [`JournalLine`].
///
/// Returns `None` for a blank line, malformed JSON, a non-object, or an entry
/// whose `MESSAGE` is not a plain string (journald renders non-UTF-8 messages as
/// an array of byte values; such binary log records carry no service transition
/// we classify, so they are skipped rather than lossily decoded). A missing
/// `_SYSTEMD_UNIT`/`SYSLOG_IDENTIFIER` is tolerated as an empty selector.
pub fn parse_line(line: &str) -> Option<JournalLine> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let obj = value.as_object()?;
    // MESSAGE must be a plain string; an array (binary message) is not a
    // transition we classify.
    let message = obj.get("MESSAGE")?.as_str()?.to_string();
    let unit = obj
        .get("_SYSTEMD_UNIT")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let identifier = obj
        .get("SYSLOG_IDENTIFIER")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some(JournalLine {
        unit,
        identifier,
        message,
    })
}

/// Which recognised service a line belongs to, or `None` for everything else.
fn service_of(unit: &str, identifier: &str) -> Option<&'static str> {
    let u = unit.to_ascii_lowercase();
    let i = identifier.to_ascii_lowercase();
    if u.starts_with("networkmanager") || i == "networkmanager" {
        Some("network")
    } else if u.starts_with("bluetooth") || i == "bluetoothd" || i == "bluetooth" {
        Some("bluetooth")
    } else if u.starts_with("systemd-logind") || i == "systemd-logind" {
        Some("session")
    } else {
        None
    }
}

/// Classify a journal line into a coarse [`ServiceEvent`], or `None`.
///
/// The mapping is intentionally narrow: only the transitions below produce an
/// event, and `detail` is always a non-sensitive coarse identifier. A line from
/// a recognised service whose message is not one of these transitions is `None`.
pub fn classify(line: &JournalLine) -> Option<ServiceEvent> {
    let service = service_of(&line.unit, &line.identifier)?;
    let event = match service {
        "network" => classify_network(&line.message)?,
        "bluetooth" => classify_bluetooth(&line.message)?,
        "session" => classify_session(&line.message)?,
        _ => return None,
    };
    Some(event)
}

/// NetworkManager device connectivity. Reads only the interface name (e.g.
/// `wlan0`) and the new connection state; never the SSID or any address.
fn classify_network(message: &str) -> Option<ServiceEvent> {
    // NM logs device transitions as:
    //   device (wlan0): state change: ip-config -> activated (reason 'none', ...)
    // We lift only the terminal up/down states; intermediate states (prepare,
    // config, ip-config) are noise and classify to None.
    let after = message.split("state change:").nth(1)?;
    let new_state = after.split("-> ").nth(1)?.split_whitespace().next()?;
    let kind = match new_state {
        "activated" => "device-up",
        "disconnected" | "unavailable" | "deactivating" | "failed" | "unmanaged" => "device-down",
        _ => return None,
    };
    let interface = interface_name(message).unwrap_or_default();
    Some(ServiceEvent {
        service: "network".to_string(),
        kind: kind.to_string(),
        detail: interface,
    })
}

/// Extract the interface name from a `device (NAME):` prefix. The interface
/// (e.g. `wlan0`) is a local device name, not an SSID, so it is safe to carry.
fn interface_name(message: &str) -> Option<String> {
    let start = message.find("device (")? + "device (".len();
    let rest = &message[start..];
    let end = rest.find(')')?;
    Some(rest[..end].to_string())
}

/// bluetoothd device connect/disconnect. Carries NO device address (the paired
/// device's MAC is left out; only the transition is recorded). Note the
/// substring trap: `"disconnected"` contains `"connected"`, so the disconnect
/// case is tested first.
fn classify_bluetooth(message: &str) -> Option<ServiceEvent> {
    let lower = message.to_ascii_lowercase();
    let kind = if lower.contains("disconnected") {
        "disconnected"
    } else if lower.contains("connected") {
        "connected"
    } else {
        return None;
    };
    Some(ServiceEvent {
        service: "bluetooth".to_string(),
        kind: kind.to_string(),
        detail: String::new(),
    })
}

/// systemd-logind session lifecycle. Carries the numeric session id (e.g. `3`),
/// not the user name, so it is a coarse non-identifying handle.
fn classify_session(message: &str) -> Option<ServiceEvent> {
    if let Some(rest) = message.strip_prefix("New session ") {
        // "New session 3 of user tim." -> id is the leading token.
        let id = rest.split_whitespace().next()?;
        return Some(ServiceEvent {
            service: "session".to_string(),
            kind: "session-opened".to_string(),
            detail: trim_trailing_dot(id),
        });
    }
    if let Some(rest) = message.strip_prefix("Removed session ") {
        // "Removed session 3." -> id is the leading token, trailing dot stripped.
        let id = rest.split_whitespace().next()?;
        return Some(ServiceEvent {
            service: "session".to_string(),
            kind: "session-closed".to_string(),
            detail: trim_trailing_dot(id),
        });
    }
    None
}

/// Strip a single trailing `.` (logind ends some messages with a period).
fn trim_trailing_dot(s: &str) -> String {
    s.strip_suffix('.').unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(unit: &str, message: &str) -> JournalLine {
        JournalLine {
            unit: unit.to_string(),
            identifier: String::new(),
            message: message.to_string(),
        }
    }

    #[test]
    fn parses_a_journalctl_json_line() {
        let raw = r#"{"_SYSTEMD_UNIT":"NetworkManager.service","SYSLOG_IDENTIFIER":"NetworkManager","MESSAGE":"hello","__REALTIME_TIMESTAMP":"1700000000000000"}"#;
        let parsed = parse_line(raw).expect("valid line parses");
        assert_eq!(parsed.unit, "NetworkManager.service");
        assert_eq!(parsed.identifier, "NetworkManager");
        assert_eq!(parsed.message, "hello");
    }

    #[test]
    fn skips_malformed_and_binary_messages() {
        assert!(parse_line("").is_none(), "blank line");
        assert!(parse_line("not json").is_none(), "malformed");
        assert!(parse_line(r#"["array"]"#).is_none(), "non-object");
        // A binary MESSAGE renders as an array of byte values, not a string.
        let binary = r#"{"MESSAGE":[104,105]}"#;
        assert!(parse_line(binary).is_none(), "binary message skipped");
        // A missing MESSAGE is not classifiable.
        assert!(parse_line(r#"{"_SYSTEMD_UNIT":"x.service"}"#).is_none());
    }

    #[test]
    fn network_activated_is_device_up_with_interface_only() {
        let l = line(
            "NetworkManager.service",
            "<info>  [1700] device (wlan0): state change: ip-config -> activated (reason 'none', sys-iface-state: 'managed')",
        );
        let ev = classify(&l).expect("activated classifies");
        assert_eq!(ev.service, "network");
        assert_eq!(ev.kind, "device-up");
        assert_eq!(ev.detail, "wlan0", "the interface name, never the SSID");
    }

    #[test]
    fn network_disconnect_is_device_down() {
        let l = line(
            "NetworkManager.service",
            "device (eth0): state change: activated -> disconnected (reason 'carrier-changed')",
        );
        let ev = classify(&l).expect("disconnected classifies");
        assert_eq!(ev.kind, "device-down");
        assert_eq!(ev.detail, "eth0");
    }

    #[test]
    fn network_intermediate_state_is_dropped() {
        let l = line(
            "NetworkManager.service",
            "device (wlan0): state change: prepare -> config (reason 'none')",
        );
        assert!(classify(&l).is_none(), "intermediate states are noise");
    }

    #[test]
    fn network_never_emits_an_ssid() {
        // Even when the message mentions a network name, the classifier only
        // ever reads the interface from "device (NAME):" and the state token.
        let l = line(
            "NetworkManager.service",
            "device (wlan0): state change: ip-config -> activated (connection 'HomeWifiSSID')",
        );
        let ev = classify(&l).expect("classifies");
        assert!(
            !ev.detail.contains("HomeWifiSSID") && !ev.kind.contains("HomeWifiSSID"),
            "no SSID leaks into any field"
        );
        assert_eq!(ev.detail, "wlan0");
    }

    #[test]
    fn bluetooth_disconnect_checked_before_connect() {
        // "disconnected" contains "connected" - the disconnect case must win.
        let dis = JournalLine {
            unit: "bluetooth.service".to_string(),
            identifier: "bluetoothd".to_string(),
            message: "Device AA:BB:CC:DD:EE:FF Disconnected".to_string(),
        };
        let ev = classify(&dis).expect("classifies");
        assert_eq!(ev.kind, "disconnected");
        assert_eq!(ev.detail, "", "no device address carried");

        let con = JournalLine {
            unit: "bluetooth.service".to_string(),
            identifier: "bluetoothd".to_string(),
            message: "Device AA:BB:CC:DD:EE:FF Connected".to_string(),
        };
        assert_eq!(classify(&con).expect("classifies").kind, "connected");
    }

    #[test]
    fn session_open_and_close_carry_only_the_id() {
        let open = line("systemd-logind.service", "New session 7 of user tim.");
        let ev = classify(&open).expect("classifies");
        assert_eq!(ev.service, "session");
        assert_eq!(ev.kind, "session-opened");
        assert_eq!(ev.detail, "7", "the session id, not the user name");

        let close = line("systemd-logind.service", "Removed session 7.");
        let ev = classify(&close).expect("classifies");
        assert_eq!(ev.kind, "session-closed");
        assert_eq!(ev.detail, "7");
    }

    #[test]
    fn logind_noise_is_dropped() {
        let l = line("systemd-logind.service", "Watching system buttons on /dev/input/event0");
        assert!(classify(&l).is_none());
        let seat = line("systemd-logind.service", "New seat seat0.");
        assert!(classify(&seat).is_none(), "seat events are not session events");
    }

    #[test]
    fn unrecognised_service_is_dropped() {
        let l = line("sshd.service", "Accepted publickey for tim");
        assert!(classify(&l).is_none(), "only the three named services classify");
    }

    #[test]
    fn identifier_fallback_selects_the_service() {
        // When _SYSTEMD_UNIT is absent, the SYSLOG_IDENTIFIER still selects.
        let l = JournalLine {
            unit: String::new(),
            identifier: "systemd-logind".to_string(),
            message: "New session 1 of user a.".to_string(),
        };
        assert_eq!(classify(&l).expect("classifies").kind, "session-opened");
    }
}
