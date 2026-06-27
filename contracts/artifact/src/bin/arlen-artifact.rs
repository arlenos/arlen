// SPDX-FileCopyrightText: 2026 Tim Kicker
// SPDX-License-Identifier: Apache-2.0

//! `arlen-artifact` - wrap a program's stdout in a typed artifact envelope.
//!
//! Usage: `<program> | arlen-artifact <KIND> [--title T] [--text T] [--lang L]
//! [--media-type M]`. The body is read from stdin; the helper builds the typed
//! payload, then emits two legs to stdout: the plain-text floor first (visible on
//! any terminal) and the APC sidecar after (decoded only by an Arlen terminal).
//! The artifact is always stamped `ExternalContent` - a program cannot assert a
//! trusted origin, and `widget` is refused.
//!
//! Exit codes: 0 ok; 2 invalid input (unknown kind, empty body with no `--text`,
//! malformed chart JSON); 1 I/O error writing the legs.

use std::io::{self, Write};
use std::process::ExitCode;
use std::str::FromStr;

use clap::Parser;

use arlen_artifact::build::{build_from_stdin, emit_legs, BuildOpts};
use arlen_artifact::ArtifactKind;

/// Wrap piped stdin in a typed artifact envelope and print both legs.
#[derive(Parser)]
#[command(name = "arlen-artifact")]
struct Cli {
    /// The artifact kind: markdown, code, table, chart, image, diagram or links
    /// (not widget).
    kind: String,
    /// An optional cosmetic title.
    #[arg(long)]
    title: Option<String>,
    /// An explicit plain-text floor. Defaults to the verbatim stdin.
    #[arg(long)]
    text: Option<String>,
    /// A language hint for `code` (free) or `diagram` (mermaid/dot).
    #[arg(long)]
    lang: Option<String>,
    /// The image media type for `image` (png, jpeg, svg, webp, gif; default png).
    #[arg(long)]
    media_type: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let kind = match ArtifactKind::from_str(&cli.kind) {
        Ok(k) => k,
        Err(_) => {
            eprintln!(
                "unknown artifact kind '{}'; known kinds: markdown, code, table, chart, image, diagram, links",
                cli.kind
            );
            return ExitCode::from(2);
        }
    };

    let opts = BuildOpts {
        text: cli.text,
        title: cli.title,
        language: cli.lang,
        media_type: cli.media_type,
    };

    let artifact = match build_from_stdin(kind, io::stdin().lock(), &opts) {
        Ok(a) => a,
        Err(e) => {
            // Invalid input: print nothing to stdout (no half-artifact), report why.
            eprintln!("arlen-artifact: {e}");
            return ExitCode::from(2);
        }
    };

    let mut stdout = io::stdout().lock();
    if let Err(e) = emit_legs(&artifact, &mut stdout).and_then(|()| stdout.flush()) {
        eprintln!("arlen-artifact: write error: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
