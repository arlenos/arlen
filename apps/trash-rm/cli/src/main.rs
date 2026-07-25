//! `trash-rm`: the trash-first `rm` (CAH-2). Parses `rm`'s flags, then routes an
//! interactive delete to the reversible freedesktop home trash (journaling a
//! restorable inverse to the undo-signer) and a scripted delete to a POSIX hard
//! unlink, so a script relying on `rm`'s exact behaviour is never silently changed.
//!
//! The shell prefers this interactively via a function or command, NEVER a PATH
//! alias named `rm` (aliasing breaks scripts and non-interactive invocations); a
//! deliberate `--purge` always hard-unlinks.

use arlen_ai_undo_core::effect_model::InverseReceipt;
use arlen_ai_undo_core::undo_log::UndoEntry;
use arlen_ai_undo_proto::{read_response, socket_path, write_request, Request};
use arlen_trash_rm_core::parse::{parse_rm_args, RmError, RmInvocation};
use arlen_trash_rm_core::route::{route_delete, DeleteMode};
use arlen_trash_rm_core::unlink::execute_unlink;
use std::io::{IsTerminal, Write};
use std::path::Path;

mod trash;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut inv = match parse_rm_args(&args) {
        Ok(inv) => inv,
        Err(e) => {
            eprintln!("trash-rm: {}", render_parse_error(&e));
            std::process::exit(2);
        }
    };

    // `-i`: confirm each operand before deleting. A declined operand is dropped from
    // the effective set. With no answer available (EOF / non-tty), treat as "no".
    if inv.interactive {
        inv.paths = confirm_operands(&inv.paths);
    }
    // Nothing left to do (all declined, or `-f` with no operands): success no-op.
    if inv.paths.is_empty() {
        std::process::exit(0);
    }

    let interactive_session = std::io::stdout().is_terminal();
    let code = match route_delete(&inv, interactive_session) {
        DeleteMode::Unlink => run_unlink(&inv),
        DeleteMode::Trash => run_trash(&inv).await,
    };
    std::process::exit(code);
}

/// Hard-unlink the operands and report per-operand failures on stderr.
fn run_unlink(inv: &RmInvocation) -> i32 {
    let report = execute_unlink(inv);
    for (path, why) in &report.errors {
        eprintln!("trash-rm: cannot remove '{path}': {why}");
    }
    if inv.verbose {
        for path in &report.removed {
            println!("removed '{path}'");
        }
    }
    report.exit_code()
}

/// Trash the operands into the home trash and journal each restorable inverse to the
/// undo-signer (best-effort: an absent or failing signer never fails a committed,
/// reversible trash).
async fn run_trash(inv: &RmInvocation) -> i32 {
    let report = trash::execute_trash(inv);
    for (path, why) in &report.errors {
        eprintln!("trash-rm: cannot trash '{path}': {why}");
    }
    let socket = socket_path();
    if socket.exists() {
        for (_path, inverse) in &report.trashed {
            if let Err(e) = journal_inverse(&socket, inverse.clone()).await {
                // The trash committed; a journal miss only loses the undo record.
                eprintln!("trash-rm: warning: could not record undo: {e}");
            }
        }
    }
    if inv.verbose {
        for (path, _) in &report.trashed {
            println!("trashed '{path}'");
        }
    }
    report.exit_code()
}

/// Submit a captured inverse to the undo-signer as a fresh `SubmitCreated` entry.
async fn journal_inverse(socket: &Path, inverse: InverseReceipt) -> Result<(), String> {
    let op_id = random_op_id();
    let entry = UndoEntry { op_id: op_id.clone(), correlation_id: op_id, inverse };
    let mut stream =
        tokio::net::UnixStream::connect(socket).await.map_err(|e| e.to_string())?;
    write_request(&mut stream, &Request::SubmitCreated(entry))
        .await
        .map_err(|e| e.to_string())?;
    // Read the ack so the signer commits before we return; ignore its content.
    let _ = read_response(&mut stream).await;
    Ok(())
}

/// A fresh 128-bit op id as lowercase hex (fits `MAX_OP_ID_LEN`; unique per trash so
/// two entries never collide on the signer's key).
fn random_op_id() -> String {
    let mut bytes = [0u8; 16];
    // A failure here is astronomically unlikely; fall back to a time-derived id so a
    // trash is still journaled rather than dropped.
    if getrandom::getrandom(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        return format!("trash-{}-{nanos}", std::process::id());
    }
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Prompt for each operand under `-i`; return the accepted subset. A line beginning
/// `y`/`Y` accepts; anything else (including EOF on a pipe) declines, so nothing is
/// removed without an explicit yes.
fn confirm_operands(paths: &[String]) -> Vec<String> {
    let mut accepted = Vec::new();
    let stdin = std::io::stdin();
    for path in paths {
        eprint!("trash-rm: remove '{path}'? ");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => break, // EOF: decline this and every remaining operand.
            Ok(_) => {
                if matches!(line.trim_start().bytes().next(), Some(b'y' | b'Y')) {
                    accepted.push(path.clone());
                }
            }
            Err(_) => break,
        }
    }
    accepted
}

/// A human-readable message for a parse failure (exit code 2, like `rm`'s usage
/// errors).
fn render_parse_error(e: &RmError) -> String {
    match e {
        RmError::UnknownFlag(f) => format!("unrecognized option '{f}'"),
        RmError::MissingOperand => "missing operand".to_string(),
        RmError::RefusedRoot => {
            "it is dangerous to operate recursively on '/' (use --no-preserve-root to override)"
                .to_string()
        }
    }
}
