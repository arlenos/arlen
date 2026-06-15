//! The typed CUPS/IPP model (printing-plan.md PRN-R1): printers, jobs, their
//! states, and the local-vs-network destination classification. Kept pure so the
//! IPP integer-to-state mapping, the queue states and the destination decision
//! are unit-tested without a running cupsd; the live IPP transport
//! ([`crate::backend`]) builds on these types.

/// A printer's operational state, from the IPP `printer-state` enum
/// (RFC 8011 §5.4.11: 3=idle, 4=processing, 5=stopped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterState {
    /// Ready, no job in progress.
    Idle,
    /// Actively printing a job.
    Processing,
    /// Stopped (an error, paused, or out of paper/ink).
    Stopped,
    /// An out-of-range value the server reported; surfaced rather than guessed.
    Unknown(i32),
}

impl PrinterState {
    /// Map the IPP `printer-state` integer.
    pub fn from_ipp(v: i32) -> Self {
        match v {
            3 => PrinterState::Idle,
            4 => PrinterState::Processing,
            5 => PrinterState::Stopped,
            other => PrinterState::Unknown(other),
        }
    }

    /// A stable lowercase key for display / audit.
    pub fn as_key(&self) -> &'static str {
        match self {
            PrinterState::Idle => "idle",
            PrinterState::Processing => "processing",
            PrinterState::Stopped => "stopped",
            PrinterState::Unknown(_) => "unknown",
        }
    }
}

/// A print job's state, from the IPP `job-state` enum (RFC 8011 §5.3.7:
/// 3=pending, 4=held, 5=processing, 6=stopped, 7=canceled, 8=aborted,
/// 9=completed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// Queued, not yet started.
    Pending,
    /// Held for a resource or by request.
    Held,
    /// Being printed.
    Processing,
    /// Stopped mid-print.
    Stopped,
    /// Cancelled by the user / system.
    Canceled,
    /// Aborted by the server after an error.
    Aborted,
    /// Finished successfully.
    Completed,
    /// An out-of-range value the server reported.
    Unknown(i32),
}

impl JobState {
    /// Map the IPP `job-state` integer.
    pub fn from_ipp(v: i32) -> Self {
        match v {
            3 => JobState::Pending,
            4 => JobState::Held,
            5 => JobState::Processing,
            6 => JobState::Stopped,
            7 => JobState::Canceled,
            8 => JobState::Aborted,
            9 => JobState::Completed,
            other => JobState::Unknown(other),
        }
    }

    /// A stable lowercase key for display / audit.
    pub fn as_key(&self) -> &'static str {
        match self {
            JobState::Pending => "pending",
            JobState::Held => "held",
            JobState::Processing => "processing",
            JobState::Stopped => "stopped",
            JobState::Canceled => "canceled",
            JobState::Aborted => "aborted",
            JobState::Completed => "completed",
            JobState::Unknown(_) => "unknown",
        }
    }

    /// Whether the job has reached a terminal state (no further transitions).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            JobState::Canceled | JobState::Aborted | JobState::Completed
        )
    }
}

/// Where a printer physically lives, from its device/printer URI. The
/// printing-plan §4 angle: a network destination means the document leaves the
/// machine over the LAN (print-as-egress), so it is shown honestly and audited
/// distinctly from a directly-attached local printer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// A directly-attached printer (USB / parallel / serial, or a queue on this
    /// host): the document never leaves the machine.
    Local,
    /// A network printer (IPP/IPPS/socket/LPD/DNS-SD to another host): printing
    /// sends the document over the network.
    Network,
}

impl Destination {
    /// A stable lowercase key for display / audit.
    pub fn as_key(&self) -> &'static str {
        match self {
            Destination::Local => "local",
            Destination::Network => "network",
        }
    }
}

/// Classify a CUPS device/printer URI as a local or network destination.
///
/// Local schemes (`usb`, `parallel`, `serial`, `hp`, `hpfax`, `file`) are always
/// directly attached. Network schemes (`ipp`, `ipps`, `http`, `https`, `socket`,
/// `lpd`, `dnssd`, `mdns`) are network UNLESS the host is loopback (a queue on
/// this host, e.g. `ipp://localhost/...`). An unrecognised or hostless URI is
/// classified `Network` - the conservative default, since misclassifying a
/// network printer as local would under-state the print-as-egress fact.
pub fn classify_destination(uri: &str) -> Destination {
    // The scheme is the part before the first ':'. CUPS uses both `scheme://host`
    // (usb, ipp, socket) and `scheme:/path` (parallel, hp, serial, file) forms,
    // so split on ':' not "://".
    let scheme = uri.split(':').next().unwrap_or("").to_ascii_lowercase();
    match scheme.as_str() {
        "usb" | "parallel" | "serial" | "hp" | "hpfax" | "file" | "direct" => Destination::Local,
        "ipp" | "ipps" | "http" | "https" | "socket" | "lpd" | "dnssd" | "mdns" => {
            match host_of(uri) {
                Some(host) if is_loopback_host(&host) => Destination::Local,
                _ => Destination::Network,
            }
        }
        // Unknown scheme: assume network (never under-state egress).
        _ => Destination::Network,
    }
}

/// Extract the host component of a `scheme://host[:port]/...` URI, lowercased.
fn host_of(uri: &str) -> Option<String> {
    let after = uri.split("://").nth(1)?;
    let authority = after.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }
    // Strip a trailing :port (but not an IPv6 colon set: keep it simple and only
    // strip a final :digits run, which a bare host:port has).
    let host = match authority.rsplit_once(':') {
        Some((h, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => authority,
    };
    Some(host.trim_matches(['[', ']']).to_ascii_lowercase())
}

/// Whether a host string is the loopback / this-host (so a queue there is local).
///
/// An IP literal is checked NUMERICALLY (`127.0.0.0/8` and `::1` are loopback,
/// per [`std::net::IpAddr::is_loopback`]); a non-IP host matches only the exact
/// loopback names. This deliberately does NOT prefix-match `127.` on the raw
/// string: a DNS host like `127.evil.com` or `127.0.0.1.evil.com` is a PUBLIC
/// name that must classify Network, not Local - misclassifying it Local would
/// hide the print-as-egress (the §4.2 boundary), so the parse-as-IP path is the
/// only way a `127.x` value is trusted as loopback.
fn is_loopback_host(host: &str) -> bool {
    // Strip an IPv6 zone id (e.g. `fe80::1%eth0`) before parsing.
    let bare = host.split('%').next().unwrap_or(host);
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    matches!(host, "localhost" | "ip6-localhost" | "localhost.localdomain")
}

/// A configured printer queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printer {
    /// The CUPS queue name (`printer-name`).
    pub name: String,
    /// The device/printer URI (`device-uri` / `printer-uri-supported`).
    pub uri: String,
    /// A human description (`printer-info`), if any.
    pub info: Option<String>,
    /// The location string (`printer-location`), if any.
    pub location: Option<String>,
    /// The make and model (`printer-make-and-model`), if any.
    pub make_model: Option<String>,
    /// Operational state.
    pub state: PrinterState,
    /// Whether the queue is accepting new jobs (`printer-is-accepting-jobs`).
    pub accepting_jobs: bool,
    /// Local vs network, classified from [`Printer::uri`].
    pub destination: Destination,
}

impl Printer {
    /// Build a printer, classifying its destination from the URI.
    pub fn new(
        name: impl Into<String>,
        uri: impl Into<String>,
        info: Option<String>,
        location: Option<String>,
        make_model: Option<String>,
        state: PrinterState,
        accepting_jobs: bool,
    ) -> Self {
        let uri = uri.into();
        let destination = classify_destination(&uri);
        Self {
            name: name.into(),
            uri,
            info,
            location,
            make_model,
            state,
            accepting_jobs,
            destination,
        }
    }
}

/// A print job in a queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The IPP `job-id`.
    pub id: i32,
    /// The queue the job belongs to (`printer-name` / from the printer-uri).
    pub printer: String,
    /// The job name / document title (`job-name`), if any.
    pub name: Option<String>,
    /// The submitting user (`job-originating-user-name`), if any.
    pub user: Option<String>,
    /// The job's state.
    pub state: JobState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printer_state_maps_known_and_unknown() {
        assert_eq!(PrinterState::from_ipp(3), PrinterState::Idle);
        assert_eq!(PrinterState::from_ipp(4), PrinterState::Processing);
        assert_eq!(PrinterState::from_ipp(5), PrinterState::Stopped);
        assert_eq!(PrinterState::from_ipp(99), PrinterState::Unknown(99));
        assert_eq!(PrinterState::Idle.as_key(), "idle");
    }

    #[test]
    fn job_state_maps_and_flags_terminal() {
        assert_eq!(JobState::from_ipp(3), JobState::Pending);
        assert_eq!(JobState::from_ipp(9), JobState::Completed);
        assert_eq!(JobState::from_ipp(0), JobState::Unknown(0));
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Canceled.is_terminal());
        assert!(JobState::Aborted.is_terminal());
        assert!(!JobState::Pending.is_terminal());
        assert!(!JobState::Processing.is_terminal());
    }

    #[test]
    fn local_schemes_classify_local() {
        assert_eq!(classify_destination("usb://HP/LaserJet?serial=42"), Destination::Local);
        assert_eq!(classify_destination("parallel:/dev/lp0"), Destination::Local);
        assert_eq!(classify_destination("hp:/usb/HP_LaserJet"), Destination::Local);
    }

    #[test]
    fn network_schemes_classify_network_unless_loopback() {
        assert_eq!(classify_destination("ipp://printer.lan:631/ipp/print"), Destination::Network);
        assert_eq!(classify_destination("ipps://192.168.1.50/ipp/print"), Destination::Network);
        assert_eq!(classify_destination("socket://10.0.0.9:9100"), Destination::Network);
        assert_eq!(classify_destination("dnssd://Office%20Printer._ipp._tcp.local/"), Destination::Network);
        // A queue on this host is local even over ipp.
        assert_eq!(classify_destination("ipp://localhost:631/printers/PDF"), Destination::Local);
        assert_eq!(classify_destination("ipp://127.0.0.1/printers/X"), Destination::Local);
        assert_eq!(classify_destination("ipp://127.0.0.2/printers/X"), Destination::Local, "all of 127/8 is loopback");
        assert_eq!(classify_destination("https://[::1]:631/ipp/print"), Destination::Local);
        assert_eq!(classify_destination("https://[0:0:0:0:0:0:0:1]/ipp/print"), Destination::Local, "long-form ::1");
    }

    #[test]
    fn a_dns_host_beginning_127_is_network_not_loopback() {
        // The egress-hiding trap: a public DNS name that merely starts `127.`
        // (or embeds the loopback IP as a subdomain) must NOT be treated as
        // loopback - that would hide the print-as-egress.
        assert_eq!(classify_destination("ipp://127.evil.com/ipp/print"), Destination::Network);
        assert_eq!(classify_destination("ipp://127.0.0.1.evil.com/ipp/print"), Destination::Network);
        assert_eq!(classify_destination("ipp://127.0.0.1456/ipp/print"), Destination::Network, "not a valid IP");
        assert_eq!(classify_destination("ipps://localhost.evil.com/ipp/print"), Destination::Network);
    }

    #[test]
    fn unknown_or_hostless_uri_defaults_to_network() {
        // Never under-state egress: an unrecognised scheme is treated as network.
        assert_eq!(classify_destination("weird://thing"), Destination::Network);
        assert_eq!(classify_destination("not-a-uri"), Destination::Network);
    }

    #[test]
    fn printer_new_classifies_its_destination() {
        let p = Printer::new(
            "Office",
            "ipp://printer.lan/ipp/print",
            Some("Front desk".into()),
            None,
            Some("Brand Model".into()),
            PrinterState::Idle,
            true,
        );
        assert_eq!(p.destination, Destination::Network);
        assert_eq!(p.state.as_key(), "idle");
        assert!(p.accepting_jobs);
    }
}
