//! The launch socket: the one place a request to start something becomes a
//! process.
//!
//! The portal used to shell out to `xdg-open`, the file manager spawned the
//! `Exec` its picker chose, and the launcher had its own copy of the
//! confinement decision - three components each holding a third of the problem
//! and none holding the part that mattered. This serves all of them:
//! `$XDG_RUNTIME_DIR/arlen/launch.sock` takes an app id or a document, resolves
//! it, records who asked, and starts it the one confined way.
//!
//! The decisions live in `arlen_desktop_shell_core::launch` and are unit-tested
//! there. This is the shell around them: bind, attest the peer, read the files,
//! submit the record, spawn.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use arlen_desktop_shell_core::launch::{
    plan::Launch,
    search::{self, XdgEnv},
    service::{self, Caller, Served},
};
use arlen_launch_contract as proto;
use arlen_permissions::ConnectionAuth;
use audit_proto::sink::{AuditSink, LedgerAuditSink};
use tokio::net::{UnixListener, UnixStream};

/// Start serving launch requests, if the socket can be had.
///
/// A shell that cannot bind still has to come up - the bar, the launcher and
/// everything else do not depend on this - so a failure is logged and the
/// service is simply absent. Callers get a connection refusal, which is a
/// legible "the service is not there" rather than a hang.
///
/// This is called from Tauri's setup hook, which is NOT inside a tokio runtime.
/// Binding with `tokio::net::UnixListener` panicked on "there is no reactor
/// running" every time the shell started - on the main thread, in a function
/// whose whole contract is that it degrades quietly - and the bare `tokio::spawn`
/// underneath it would have panicked next for the same reason. The three sibling
/// IPC services in the same hook (clipboard, intent, search) all go through
/// `tauri::async_runtime::spawn`, which is the runtime this process has; this one
/// had grown its own spelling.
///
/// The socket is bound here rather than inside the task, so that when `setup`
/// returns the socket exists. A caller that connects the moment the shell is up
/// then gets served rather than refused, which the siblings do not guarantee.
pub fn spawn_launch_service() {
    let path = proto::socket_path();
    let listener = match bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::error!("launch service not started: {e}");
            return;
        }
    };
    if let Err(e) = listener.set_nonblocking(true) {
        log::error!("launch service not started: {e}");
        return;
    }
    log::info!("launch service listening on {}", path.display());
    tauri::async_runtime::spawn(async move {
        // Inside the runtime, so this is the first line that may touch tokio.
        let listener = match UnixListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                log::error!("launch service not started: {e}");
                return;
            }
        };
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = handle(stream).await {
                            log::warn!("launch request: {e}");
                        }
                    });
                }
                Err(e) => {
                    log::error!("launch service accept failed, stopping: {e}");
                    return;
                }
            }
        }
    });
}

/// Bind the socket, owner-only.
///
/// A path that is already served by a live process is an error rather than
/// something to take over: two shells answering one socket would hand the same
/// request two different answers. A stale file left by a dead one is cleared,
/// which is the ordinary case after a crash.
fn bind(path: &Path) -> std::io::Result<std::os::unix::net::UnixListener> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path.exists() {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                format!("{} is already served by a live process", path.display()),
            ));
        }
        let _ = std::fs::remove_file(path);
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Serve one request off the socket.
async fn handle(mut stream: UnixStream) -> Result<(), String> {
    let caller = attest(&stream);
    let request = proto::read_message(&mut stream)
        .await
        .map_err(|e| e.to_string())?;
    match request {
        proto::Request::Launch(request) => {
            let outcome = dispatch(&request, &caller).await;
            proto::write_outcome(&mut stream, &outcome)
                .await
                .map_err(|e| e.to_string())
        }
        proto::Request::Query(query) => {
            let answer = answer_query(&query, &caller);
            proto::write_answer(&mut stream, &answer)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

/// The shell's own name, for a launch the shell itself asks for.
///
/// Matches what `arlen_permissions` resolves the shell's binary to, so a line in
/// the ledger reads the same whether the request arrived over the socket from
/// another process or came from a menu item in this one. A separate label like
/// "internal" would make the shell's own launches the one kind nobody can grep
/// for by application.
const SELF_CALLER: &str = "desktop-shell";

/// The shell asking on its own behalf.
///
/// A constructor rather than a public `Caller`, because the only honest thing an
/// in-process caller can be is this one. Handing out the type would let any call
/// site name any application as the cause of a launch, which is the exact claim
/// the socket refuses to take on trust from a peer.
pub fn self_caller() -> Caller {
    Caller::Named(SELF_CALLER.to_string())
}

/// Decide, record and perform one launch, whatever asked for it.
///
/// Split out of [`handle`] so the shell's own call sites - the waypointer, the
/// recent-files list, an intent handoff - reach the launch path without dialling
/// a socket they are already serving. They used to spawn `xdg-open` instead,
/// which meant the shell's own launches were the ones with no audit line, no
/// resolved app id and no confinement: exactly the launches an attacker would
/// most like to be invisible, missing from the ledger because the code that
/// records them was on the other side of a socket the shell never called.
pub async fn dispatch(request: &proto::LaunchRequest, caller: &Caller) -> proto::LaunchOutcome {
    let env = XdgEnv::from_process();
    let handlers = search::load_mimeapps(&env);
    let confined = crate::shell_config::get_shell_config()
        .map(|c| c.launcher.confined)
        .unwrap_or(false);

    let served = service::serve(
        request,
        caller,
        &handlers,
        |id| search::load_entry(&env, id),
        mime_of,
        confined,
        // Per-app confinement, same rule and same source as the launcher's:
        // `profile_paths` lists every path the loader consults, so an app the
        // system holds no profile for is not routed through `arlen-run` and
        // cannot be stopped by turning the flag on.
        |id| {
            arlen_permissions::profile_paths(id)
                .iter()
                .any(|p| p.exists())
        },
    );

    // Before the act, not after: a record written afterwards is a record that a
    // crash between the two loses, and the one question this socket exists to
    // answer is what caused a program to start.
    record(&served).await;

    // A failed spawn is ANSWERED, not dropped. This used to be `spawn(launch)?`,
    // which returned before the write and left the caller with a closed
    // connection - unable to tell "nothing opens this" from "it did not start"
    // from "the shell died". The reason travels with it because the caller shows
    // it to a person.
    let outcome = match (&served.launch, &served.outcome) {
        (Some(launch), proto::LaunchOutcome::Started { app_id }) => match spawn(launch) {
            Ok(()) => served.outcome.clone(),
            Err(reason) => {
                log::warn!("launch request: {reason}");
                proto::LaunchOutcome::DidNotStart {
                    app_id: app_id.clone(),
                    reason,
                }
            }
        },
        _ => served.outcome.clone(),
    };
    outcome
}

/// What kind of thing a document is, when the caller did not say.
///
/// `xdg-mime` rather than a MIME library: it is what the rest of the tree
/// already asks (the harness's file pills, the file manager's Open-With), so a
/// user who has taught their system that a file is one thing gets one answer
/// everywhere rather than two. A remote target has no local path to classify,
/// and a classification nobody could make is `None` - which the service reports
/// as nothing opening it, because from the requester's side that is the same
/// fact.
/// The path is canonicalized first, and that is load-bearing rather than tidy.
/// `xdg-mime` parses a leading-dash argument as its own option - measured 12 Aug:
/// `xdg-mime query filetype -x.txt` answers "unexpected option '-x.txt'" - and
/// this path arrives over the launch socket from another app, so it is whatever
/// that app sent. A file called `-notes.txt` in the requester's working directory
/// would have failed classification here and been reported as `NoHandler`: the
/// wrong error, about a file that has a perfectly good handler. The tree's other
/// two `xdg-mime` callers already pass an absolute path; this was the one that
/// did not.
fn mime_of(target: &proto::Target) -> Option<String> {
    let path = target.path.as_ref()?;
    let path = std::fs::canonicalize(path).ok()?;
    Some(mime_db().guess_mime_type().path(&path).guess().mime_type().to_string())
}

/// Answer a mime query, or refuse it.
///
/// The refusal is the point. *What kind of file is this* leaks that a path exists
/// and what it is, so it is answered only for paths this caller could have opened.
/// An unnamed caller holds no profile and so reaches nothing - the honest answer
/// when there is no grant to check is no.
fn answer_query(query: &proto::MimeQuery, caller: &Caller) -> proto::MimeAnswer {
    let Caller::Named(app_id) = caller else {
        return proto::MimeAnswer::Refused {
            reason: "the caller could not be identified, so no grant could be checked".into(),
        };
    };
    // Canonical FIRST, then checked. `~/Documents/../.ssh/id_rsa` is inside a
    // documents grant only until the `..` is resolved, and a check on the string
    // as sent would have admitted it.
    let Ok(path) = std::fs::canonicalize(&query.path) else {
        // Absent and unreadable answer the same way. A caller that could tell
        // "no such file" from "not yours" would have a probe for what exists
        // outside its reach, which is the thing the gate is for.
        return proto::MimeAnswer::Refused {
            reason: "not a readable path for this application".into(),
        };
    };
    let profile = arlen_permissions::load_profile(app_id);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let dirs = user_dirs(&home);
    let readable = profile
        .map(|p| p.filesystem.readable_dirs(&home, &dirs))
        .unwrap_or_default();
    if !is_within(&path, &readable) {
        return proto::MimeAnswer::Refused {
            reason: "not a readable path for this application".into(),
        };
    }
    match mime_of(&proto::Target {
        uri: String::new(),
        path: Some(path.to_string_lossy().into_owned()),
    }) {
        Some(mime) => proto::MimeAnswer::Type { mime },
        None => proto::MimeAnswer::Unknown,
    }
}

/// Whether a canonical path lies inside any granted directory.
///
/// `Path::starts_with` compares WHOLE COMPONENTS, which is the entire reason this
/// is not a string prefix test: `/home/u/documents-private` starts with the text
/// of a `/home/u/documents` grant and is a different directory. A string compare
/// here would hand out the type of every sibling directory whose name happens to
/// share a prefix with something granted.
fn is_within(path: &std::path::Path, granted: &[std::path::PathBuf]) -> bool {
    granted.iter().any(|d| path.starts_with(d))
}

/// The XDG user directories, as the launcher resolves them.
fn user_dirs(home: &std::path::Path) -> arlen_permissions::UserDirs {
    arlen_permissions::UserDirs {
        documents: home.join("Documents"),
        downloads: home.join("Downloads"),
        pictures: home.join("Pictures"),
        music: home.join("Music"),
        videos: home.join("Videos"),
    }
}

/// The shared-mime-info database, read once.
///
/// **In process, not `xdg-mime query filetype`.** The point of moving every opener
/// onto this socket was that the desktop stops asking a shell script about its own
/// files; leaving the one authoritative resolver shelling out would have left every
/// answer depending on `xdg-utils` being installed and behaving. It also removes a
/// fork per launch from the path a person waits on.
///
/// Loaded once because building it parses every `globs2`, `magic` and alias table
/// under the data dirs - per call that is a real cost on a path that runs on every
/// double-click. The trade is that a mime database installed after the shell
/// started is not seen until it restarts, which is the same trade the handler cache
/// already makes and is why this is a `OnceLock` rather than a reload-on-change.
fn mime_db() -> &'static xdg_mime::SharedMimeInfo {
    static DB: std::sync::OnceLock<xdg_mime::SharedMimeInfo> = std::sync::OnceLock::new();
    DB.get_or_init(xdg_mime::SharedMimeInfo::new)
}

/// Who is on the other end, as the kernel says rather than as they claim.
///
/// A peer whose binary the identity resolver does not recognise is `Unnamed`,
/// not refused: most of what a session runs is not a packaged application, and
/// opening a document grants nothing that would justify turning that into a
/// failure. The distinction is recorded rather than smoothed over.
fn attest(stream: &UnixStream) -> Caller {
    // Safe: `getuid` cannot fail and touches no shared state.
    let uid = unsafe { libc::getuid() };
    match ConnectionAuth::extract_from(stream, uid) {
        Ok(auth) => match auth.verify_alive() {
            Ok(()) => Caller::Named(auth.app_id().to_string()),
            // The peer went away between connecting and being asked about, so
            // the name we would record might already belong to someone else.
            Err(_) => Caller::Unnamed,
        },
        Err(_) => Caller::Unnamed,
    }
}

/// Submit the ledger record.
///
/// Best effort **for now**, and deliberately not silent: a launch grants the
/// caller nothing it could not do by spawning the program itself, so refusing
/// to open a document because the audit daemon is down would cost the user a
/// working desktop to protect a record of something they could have done
/// anyway. That reasoning expires with the confinement flip, when this socket
/// becomes a confined application's only way to start anything and the record
/// becomes the account of a real authority - at which point this fails closed,
/// in one place, like `service::admits`.
async fn record(served: &Served) {
    let sink = LedgerAuditSink::at_default_socket();

    // The daemon is back and something was lost while it was away: say so in the
    // ledger before the entry that proves it is back, so a reader meets the gap
    // in the order it happened. Left pending if this fails too - a gap that
    // cannot be written is not a gap that stops existing.
    if let Some(gap) = take_gap() {
        let marker = service::unrecorded_gap_event(gap.dropped, gap.span_ms());
        if sink.submit(marker).await.is_err() {
            restore_gap(gap);
        }
    }

    let event = service::launch_event(&served.audit);
    if let Err(e) = sink.submit(event).await {
        let dropped = note_drop();
        log::warn!(
            "launch not recorded ({}): caller {} outcome {} ({} unrecorded so far)",
            e,
            served.audit.caller,
            served.audit.outcome,
            dropped
        );
    }
}

/// Launches that went unrecorded, and when the first of them was.
#[derive(Clone, Copy)]
struct Gap {
    dropped: u64,
    since: std::time::Instant,
}

impl Gap {
    fn span_ms(&self) -> u64 {
        u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX)
    }
}

/// The pending gap, if the ledger is currently missing anything.
///
/// A process-global because there is one launch service per shell and one
/// ledger behind it; threading it through every request would be ceremony
/// around a single fact about the process.
static GAP: std::sync::Mutex<Option<Gap>> = std::sync::Mutex::new(None);

/// Count one unrecorded launch. Returns the running total.
fn note_drop() -> u64 {
    let mut guard = GAP.lock().unwrap_or_else(|e| e.into_inner());
    let gap = guard.get_or_insert(Gap {
        dropped: 0,
        since: std::time::Instant::now(),
    });
    gap.dropped += 1;
    gap.dropped
}

/// Take the pending gap, if there is one.
fn take_gap() -> Option<Gap> {
    GAP.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Put a gap back after failing to record it, keeping the earlier start time and
/// any drops counted while the marker was in flight.
fn restore_gap(gap: Gap) {
    let mut guard = GAP.lock().unwrap_or_else(|e| e.into_inner());
    match guard.as_mut() {
        Some(current) => {
            current.dropped += gap.dropped;
            current.since = current.since.min(gap.since);
        }
        None => *guard = Some(gap),
    }
}

/// Start the planned argv, detached.
///
/// No shell: the argv came from the desktop-entry rules, and putting it back
/// through one would re-interpret a file name that has already been decided to
/// be data.
fn spawn(launch: &Launch) -> Result<(), String> {
    let argv = launch.argv();
    let (program, args) = argv.split_first().ok_or("nothing to run")?;
    std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {program}: {e}"))?;
    Ok(())
}


#[cfg(test)]
mod mime_tests {
    use super::*;

    /// Measured against `xdg-mime query filetype` on 12 Aug, five real files.
    ///
    /// Three of the five DISAGREE, and in all three the crate follows the
    /// shared-mime-info algorithm and the tool does not: `README.md` is
    /// `text/markdown` and `index.theme` is `application/x-theme` by their globs,
    /// where the tool answered `text/plain` for both. The third is the same rule
    /// pointing somewhere unwelcome - our own `arlen.raw` disk image globs to
    /// `image/x-panasonic-rw`, because `.raw` belongs to camera files and the
    /// image borrows the extension. A glob match with no contradicting magic is
    /// the answer per spec; the tool's `application/octet-stream` there comes from
    /// consulting content first, which is its own shortcut rather than the rule.
    ///
    /// Recorded because "we swapped the resolver and nothing changed" would have
    /// been the comfortable claim and it is not true.
    #[test]
    fn a_known_extension_resolves_from_the_database() {
        let readme = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../README.md");
        if !std::path::Path::new(readme).exists() {
            // Loud rather than silently passing: this test means nothing if its
            // subject moved, and a green tick over a missing file is the shape
            // this tree spent the night removing.
            panic!("{readme} is missing; the path this test resolves has moved");
        }
        let guess = mime_db().guess_mime_type().path(readme).guess();
        assert_eq!(guess.mime_type().to_string(), "text/markdown");
    }

    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_not_inside_the_grant() {
        // The bug a string comparison would have. Both of these start with the
        // TEXT of the grant; only one is inside it.
        let granted = vec![std::path::PathBuf::from("/home/u/documents")];
        assert!(is_within(std::path::Path::new("/home/u/documents/a.txt"), &granted));
        assert!(!is_within(
            std::path::Path::new("/home/u/documents-private/a.txt"),
            &granted
        ));
    }

    #[test]
    fn nothing_is_inside_an_empty_grant() {
        // An app with no profile reaches nothing. The empty case has to be a
        // refusal rather than a vacuous pass, which is what `any` over an empty
        // list gives - pinned because the opposite is one `!` away.
        assert!(!is_within(std::path::Path::new("/home/u/a.txt"), &[]));
    }

    #[test]
    fn a_target_without_a_local_path_has_no_type_to_read() {
        // A remote document is not on this filesystem, so there is nothing to
        // classify and the service asks the handler tables instead.
        let target = proto::Target { uri: "https://example.invalid/x".into(), path: None };
        assert!(mime_of(&target).is_none());
    }
}
