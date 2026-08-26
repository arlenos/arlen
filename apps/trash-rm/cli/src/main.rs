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

/// What `--help` prints.
///
/// Ordered so the two things a person needs first come first: this deletes to the
/// trash rather than destroying, and `--purge` is the flag that does destroy. The
/// rest mirrors `rm`, which is the point of the tool.
const USAGE: &str = "\
Usage: trash-rm [OPTION]... [FILE]...

Move each FILE to the trash, where it can be restored. This is a drop-in for
`rm`: the same options mean the same things.

  -r, -R, --recursive   delete directories and their contents
  -d, --dir             delete empty directories
  -f, --force           ignore files that do not exist, never prompt
  -i                    prompt before each delete
  -v, --verbose         say what is being deleted
      --one-file-system stay on one filesystem when recursing
      --no-preserve-root  do not refuse to act on `/` (the default refuses)
      --purge           DESTROY instead of trashing: unlink, no restore
      --help            show this and exit
      --version         show the version and exit
  --                    end of options; everything after is a filename

Without --purge nothing is destroyed, so a mistake is recoverable from the
trash. With it, nothing is.
";

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

    // Before anything else, and before the missing-operand refusal: `rm --help`
    // works with no operands and so must this. Printed to stdout and exit 0,
    // which is what a `--help` is for and what a caller piping it expects.
    if inv.help {
        print!("{USAGE}");
        std::process::exit(0);
    }
    if inv.version {
        println!("trash-rm {}", env!("CARGO_PKG_VERSION"));
        std::process::exit(0);
    }

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
///
/// SAID ONCE, AND SAID IN BOTH CASES. A failing signer used to warn per file and
/// an ABSENT one said nothing at all - the socket-exists test skipped the loop
/// and the run finished quietly. So the commoner of the two situations, a user
/// service that is simply not enabled, was the silent one: files went to the
/// trash and never appeared in the undo history, and nothing on the terminal
/// said so.
///
/// Both now produce one line at the end of the run rather than one per file,
/// which is also less noise than the old failing path made on a multi-file
/// delete. What it does NOT claim is that anything is lost: the freedesktop
/// trash carries its own restore info, so the file is recoverable from the file
/// manager either way. The undo HISTORY is what missed it, and that is what the
/// line says.
async fn run_trash(inv: &RmInvocation) -> i32 {
    let report = trash::execute_trash(inv);
    for (path, why) in &report.errors {
        eprintln!("trash-rm: cannot trash '{path}': {why}");
    }
    let socket = socket_path();
    let mut unrecorded = 0usize;
    if socket.exists() {
        for (_path, inverse) in &report.trashed {
            if let Err(e) = journal_inverse(&socket, inverse.clone()).await {
                // The trash committed; a journal miss only loses the undo record.
                eprintln!("trash-rm: warning: could not record undo: {e}");
                unrecorded += 1;
            }
        }
    } else {
        unrecorded = report.trashed.len();
    }
    if unrecorded > 0 {
        eprintln!(
            "trash-rm: {}",
            unrecorded_note(unrecorded, report.trashed.len())
        );
    }
    if inv.verbose {
        for (path, _) in &report.trashed {
            println!("trashed '{path}'");
        }
    }
    report.exit_code()
}

/// The one line about what the undo history did not get.
///
/// Names the count against the total rather than saying "some", because "1 of 1"
/// and "1 of 30" are different situations and the reader is the one who knows
/// which matters. Pure, so the wording is testable without a signer.
fn unrecorded_note(unrecorded: usize, trashed: usize) -> String {
    if unrecorded == trashed {
        format!(
            "the undo history did not record {} \
             (still in the trash, restorable from the file manager)",
            if trashed == 1 { "this delete".to_string() } else { format!("these {trashed} deletes") }
        )
    } else {
        format!(
            "the undo history did not record {unrecorded} of {trashed} deletes \
             (still in the trash, restorable from the file manager)"
        )
    }
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

#[cfg(test)]
mod tests {
    use super::unrecorded_note;

    /// One delete and one miss is the ordinary case: no signer running, one file
    /// gone to the trash. It must not read as a loss - the freedesktop trash
    /// holds its own restore info, so the file is there either way, and the
    /// sentence has to say which thing missed it.
    #[test]
    fn a_single_unrecorded_delete_names_the_trash_as_the_answer() {
        let s = unrecorded_note(1, 1);
        assert!(s.contains("this delete"), "{s}");
        assert!(s.contains("restorable from the file manager"), "{s}");
        assert!(!s.contains("of 1"), "a whole-run miss should not count against itself: {s}");
    }

    /// All of many. Plural, and still one line for the run rather than one per
    /// file - which is the whole reason this moved out of the loop.
    #[test]
    fn every_delete_unrecorded_reads_as_the_whole_run() {
        let s = unrecorded_note(30, 30);
        assert!(s.contains("these 30 deletes"), "{s}");
    }

    /// A partial miss is a different situation from a whole one, and "some"
    /// would flatten them. The reader is the one who knows whether 1 of 30
    /// matters.
    #[test]
    fn a_partial_miss_gives_both_numbers() {
        let s = unrecorded_note(1, 30);
        assert!(s.contains("1 of 30"), "{s}");
    }
}
