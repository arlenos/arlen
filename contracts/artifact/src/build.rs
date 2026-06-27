// SPDX-FileCopyrightText: 2026 Tim Kicker
// SPDX-License-Identifier: Apache-2.0

//! The `arlen-artifact` helper's stdin-to-artifact logic (terminal.md §3).
//!
//! The helper turns a program's raw stdout (piped into it) into a typed artifact,
//! then emits two legs: the plain-text floor first (so a pager or a non-Arlen
//! terminal shows readable output) and the APC sidecar after (invisible to a
//! terminal that does not speak it). The IPC-free core lives here as
//! [`build_from_stdin`] + [`emit_legs`] so it is unit-testable without spawning a
//! process; the binary (`src/bin/arlen-artifact.rs`) is a thin clap wrapper.
//!
//! The build path always finishes through [`Artifact::receive`], so a
//! helper-produced artifact is ALWAYS [`crate::ArtifactOrigin::ExternalContent`]
//! and a non-program-emittable kind (`widget`) is refused by the same call.

use std::io::{Read, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::osc;
use crate::{
    Artifact, ArtifactError, ArtifactKind, ArtifactPayload, ChartType, DiagramLanguage,
    ImageMediaType, Link, Series,
};

/// Options gathered from the helper's CLI flags. The body always comes from stdin;
/// these refine how it is interpreted and what the plain-text floor is.
#[derive(Debug, Default, Clone)]
pub struct BuildOpts {
    /// An explicit plain-text floor. If `None`, the verbatim stdin is used as the
    /// floor, so a dumb terminal and a copy both yield the original bytes.
    pub text: Option<String>,
    /// An optional cosmetic title.
    pub title: Option<String>,
    /// A language hint for `code` (free string) or `diagram` (`mermaid`/`dot`).
    pub language: Option<String>,
    /// The image media type for `image` (default `png`).
    pub media_type: Option<String>,
}

/// Read a program's stdout from `reader` and build the typed artifact for `kind`.
/// Always stamps `ExternalContent` (via [`Artifact::receive`]); `widget` (and any
/// non-program-emittable kind) is refused. The default plain-text floor is the
/// verbatim stdin so the fallback can never be forgotten.
pub fn build_from_stdin<R: Read>(
    kind: ArtifactKind,
    mut reader: R,
    opts: &BuildOpts,
) -> Result<Artifact, ArtifactError> {
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| ArtifactError::Malformed(format!("read stdin: {e}")))?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    // The floor defaults to the verbatim stdin (terminal.md §365: a copy yields the
    // original bytes).
    let text = opts.text.clone().unwrap_or_else(|| body.clone());

    let payload = match kind {
        ArtifactKind::Markdown => ArtifactPayload::Markdown { source: body },
        ArtifactKind::Code => ArtifactPayload::Code {
            source: body,
            language: opts.language.clone(),
        },
        ArtifactKind::Table => parse_table(&body),
        ArtifactKind::Links => parse_links(&body),
        ArtifactKind::Image => ArtifactPayload::Image {
            media_type: parse_media_type(opts.media_type.as_deref())?,
            data_base64: STANDARD.encode(&bytes),
        },
        ArtifactKind::Diagram => ArtifactPayload::Diagram {
            language: parse_diagram_language(opts.language.as_deref())?,
            source: body,
        },
        ArtifactKind::Chart => parse_chart(&bytes)?,
        ArtifactKind::Widget => return Err(ArtifactError::NotProgramEmittable(ArtifactKind::Widget)),
    };

    Artifact::receive(payload, text, opts.title.clone())
}

/// Emit the two legs to `out`: the plain-text floor (followed by a newline) FIRST,
/// then the APC sidecar frame(s). Ordering is load-bearing - a non-Arlen terminal
/// shows the text immediately and silently swallows the trailing APC.
pub fn emit_legs<W: Write>(artifact: &Artifact, out: &mut W) -> std::io::Result<()> {
    out.write_all(artifact.text.as_bytes())?;
    out.write_all(b"\n")?;
    out.write_all(&osc::encode_frames(&artifact.to_json()))?;
    Ok(())
}

/// Parse whitespace-delimited stdin into a table. The first non-empty line is the
/// header (the columns); each subsequent non-empty line is a row, split on runs of
/// whitespace. A row is padded with empty cells if short and truncated if long, so
/// the grid is always rectangular. This is dead-simple by design (terminal.md §319,
/// `ps aux | arlen-artifact table`): a column whose values contain spaces (e.g. the
/// `ps` COMMAND column) is over-split and its tail truncated - the author wanting
/// exact columns passes structured input, not free `ps` output.
fn parse_table(body: &str) -> ArtifactPayload {
    let mut lines = body.lines().filter(|l| !l.trim().is_empty());
    let columns: Vec<String> = match lines.next() {
        Some(header) => header.split_whitespace().map(|s| s.to_string()).collect(),
        None => Vec::new(),
    };
    let width = columns.len();
    let rows: Vec<Vec<String>> = lines
        .map(|line| {
            let mut cells: Vec<String> =
                line.split_whitespace().map(|s| s.to_string()).collect();
            cells.resize(width, String::new());
            cells
        })
        .collect();
    ArtifactPayload::Table { columns, rows }
}

/// Parse one link per non-empty line: `url` or `url<TAB>label`.
fn parse_links(body: &str) -> ArtifactPayload {
    let links = body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| match line.split_once('\t') {
            Some((href, label)) => Link {
                href: href.trim().to_string(),
                label: Some(label.trim().to_string()),
            },
            None => Link {
                href: line.trim().to_string(),
                label: None,
            },
        })
        .collect();
    ArtifactPayload::Links { links }
}

/// Parse the `--media-type` flag (default `png`) into the closed enum.
fn parse_media_type(s: Option<&str>) -> Result<ImageMediaType, ArtifactError> {
    match s.unwrap_or("png") {
        "png" => Ok(ImageMediaType::Png),
        "jpeg" => Ok(ImageMediaType::Jpeg),
        "svg" => Ok(ImageMediaType::Svg),
        "webp" => Ok(ImageMediaType::Webp),
        "gif" => Ok(ImageMediaType::Gif),
        other => Err(ArtifactError::Malformed(format!(
            "unknown image media type: {other}"
        ))),
    }
}

/// Parse the `--lang` flag for a diagram (default `mermaid`) into the closed enum.
fn parse_diagram_language(s: Option<&str>) -> Result<DiagramLanguage, ArtifactError> {
    match s.unwrap_or("mermaid") {
        "mermaid" => Ok(DiagramLanguage::Mermaid),
        "dot" => Ok(DiagramLanguage::Dot),
        other => Err(ArtifactError::Malformed(format!(
            "unknown diagram language: {other}"
        ))),
    }
}

/// The JSON shape a `chart` artifact accepts on stdin: a chart type plus the
/// labelled numeric series. Structured input is required because a chart cannot be
/// inferred from free text.
#[derive(serde::Deserialize)]
struct ChartInput {
    chart_type: ChartType,
    series: Vec<Series>,
}

/// Parse stdin JSON into a chart payload. Non-finite values are caught later by
/// `Artifact::receive` -> `validate_payload`.
fn parse_chart(bytes: &[u8]) -> Result<ArtifactPayload, ArtifactError> {
    let input: ChartInput = serde_json::from_slice(bytes)
        .map_err(|e| ArtifactError::Malformed(format!("chart input must be JSON: {e}")))?;
    Ok(ArtifactPayload::Chart {
        chart_type: input.chart_type,
        series: input.series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArtifactOrigin;

    const PS_OUTPUT: &str = "USER PID %CPU %MEM COMMAND\n\
root 1 0.0 0.1 /sbin/init\n\
tim 4242 1.5 2.3 cargo build\n";

    fn opts() -> BuildOpts {
        BuildOpts::default()
    }

    #[test]
    fn ps_output_roundtrips_to_table() {
        let art = build_from_stdin(ArtifactKind::Table, PS_OUTPUT.as_bytes(), &opts()).unwrap();
        assert_eq!(art.meta.origin, ArtifactOrigin::ExternalContent);
        // The floor is the verbatim stdin.
        assert_eq!(art.text, PS_OUTPUT);
        match art.payload {
            ArtifactPayload::Table { columns, rows } => {
                assert_eq!(columns, ["USER", "PID", "%CPU", "%MEM", "COMMAND"]);
                assert_eq!(rows.len(), 2);
                // The first row is padded/truncated to the 5-column width.
                assert_eq!(rows[0].len(), 5);
                assert_eq!(rows[0][0], "root");
                assert_eq!(rows[0][1], "1");
                // The COMMAND column's space-bearing value is over-split and the
                // tail truncated to the column count (the documented simple rule).
                assert_eq!(rows[1][4], "cargo");
            }
            other => panic!("expected table, got {other:?}"),
        }
    }

    #[test]
    fn ragged_short_row_is_padded() {
        let body = "a b c\n1 2\n";
        let art = build_from_stdin(ArtifactKind::Table, body.as_bytes(), &opts()).unwrap();
        if let ArtifactPayload::Table { rows, .. } = art.payload {
            assert_eq!(rows[0], vec!["1", "2", ""]);
        } else {
            panic!("expected table");
        }
    }

    #[test]
    fn explicit_text_overrides_the_floor() {
        let mut o = opts();
        o.text = Some("custom floor".into());
        let art = build_from_stdin(ArtifactKind::Markdown, b"# hi".as_slice(), &o).unwrap();
        assert_eq!(art.text, "custom floor");
    }

    #[test]
    fn empty_stdin_no_text_is_missing_text() {
        let err = build_from_stdin(ArtifactKind::Markdown, b"".as_slice(), &opts()).unwrap_err();
        assert_eq!(err, ArtifactError::MissingText);
    }

    #[test]
    fn widget_kind_is_refused() {
        let err = build_from_stdin(ArtifactKind::Widget, b"x".as_slice(), &opts()).unwrap_err();
        assert_eq!(err, ArtifactError::NotProgramEmittable(ArtifactKind::Widget));
    }

    #[test]
    fn links_parse_optional_labels() {
        let body = "https://a.example\nhttps://b.example\tB site\n";
        let art = build_from_stdin(ArtifactKind::Links, body.as_bytes(), &opts()).unwrap();
        if let ArtifactPayload::Links { links } = art.payload {
            assert_eq!(links.len(), 2);
            assert_eq!(links[0].label, None);
            assert_eq!(links[1].label.as_deref(), Some("B site"));
        } else {
            panic!("expected links");
        }
    }

    #[test]
    fn chart_parses_json_stdin() {
        let json = br#"{"chart_type":"bar","series":[{"name":"s","values":[1.0,2.0]}]}"#;
        let art = build_from_stdin(ArtifactKind::Chart, json.as_slice(), &opts()).unwrap();
        assert!(matches!(art.payload, ArtifactPayload::Chart { .. }));
        // A non-JSON chart body is a clean rejection.
        assert!(build_from_stdin(ArtifactKind::Chart, b"not json".as_slice(), &opts()).is_err());
    }

    #[test]
    fn diagram_lang_defaults_and_rejects_unknown() {
        let art = build_from_stdin(ArtifactKind::Diagram, b"graph TD".as_slice(), &opts()).unwrap();
        assert!(matches!(
            art.payload,
            ArtifactPayload::Diagram {
                language: DiagramLanguage::Mermaid,
                ..
            }
        ));
        let mut o = opts();
        o.language = Some("nonsense".into());
        assert!(build_from_stdin(ArtifactKind::Diagram, b"x".as_slice(), &o).is_err());
    }

    #[test]
    fn helper_prints_text_leg_before_the_sidecar() {
        let art = build_from_stdin(ArtifactKind::Markdown, b"plain body".as_slice(), &opts()).unwrap();
        let mut out = Vec::new();
        emit_legs(&art, &mut out).unwrap();
        // The text leg appears before the first APC introducer.
        let apc_at = find(&out, osc::APC).expect("an APC frame is emitted");
        let text_at = find(&out, b"plain body").expect("the text leg is present");
        assert!(text_at < apc_at, "the text leg must precede the sidecar");
    }

    #[test]
    fn non_arlen_terminal_sees_only_text() {
        // Strip every ESC_ .. ESC\ span (what a terminal that ignores APC does) and
        // assert the remainder is exactly the text leg.
        let art = build_from_stdin(ArtifactKind::Table, PS_OUTPUT.as_bytes(), &opts()).unwrap();
        let mut out = Vec::new();
        emit_legs(&art, &mut out).unwrap();
        let stripped = strip_apc(&out);
        assert_eq!(stripped, format!("{PS_OUTPUT}\n").into_bytes());
    }

    /// Remove every `ESC _ ... ESC \` span (simulating a terminal that discards APC).
    fn strip_apc(stream: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < stream.len() {
            if stream[i..].starts_with(osc::APC) {
                if let Some(rel) = find(&stream[i + osc::APC.len()..], osc::ST) {
                    i = i + osc::APC.len() + rel + osc::ST.len();
                    continue;
                }
            }
            out.push(stream[i]);
            i += 1;
        }
        out
    }

    fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|w| w == needle)
    }

    #[test]
    fn media_type_parses_all_known_and_defaults_to_png() {
        assert_eq!(parse_media_type(None).unwrap(), ImageMediaType::Png);
        assert_eq!(parse_media_type(Some("png")).unwrap(), ImageMediaType::Png);
        assert_eq!(parse_media_type(Some("jpeg")).unwrap(), ImageMediaType::Jpeg);
        assert_eq!(parse_media_type(Some("svg")).unwrap(), ImageMediaType::Svg);
        assert_eq!(parse_media_type(Some("webp")).unwrap(), ImageMediaType::Webp);
        assert_eq!(parse_media_type(Some("gif")).unwrap(), ImageMediaType::Gif);
        assert!(matches!(
            parse_media_type(Some("bmp")),
            Err(ArtifactError::Malformed(_))
        ));
    }

    #[test]
    fn image_build_base64_encodes_raw_bytes_and_honours_media_type() {
        // Raw bytes (incl. non-UTF-8) are base64'd verbatim, not lossily stringified.
        let raw: &[u8] = b"\x89PNG\r\n\x1a\n\xff\xfe body";
        let mut o = opts();
        o.media_type = Some("svg".to_string());
        let art = build_from_stdin(ArtifactKind::Image, raw, &o).unwrap();
        assert_eq!(art.meta.origin, ArtifactOrigin::ExternalContent);
        match art.payload {
            ArtifactPayload::Image {
                media_type,
                ref data_base64,
            } => {
                assert_eq!(media_type, ImageMediaType::Svg);
                assert_eq!(STANDARD.decode(data_base64).unwrap(), raw);
            }
            other => panic!("expected an Image payload, got {other:?}"),
        }
    }

    #[test]
    fn image_build_rejects_an_unknown_media_type() {
        let mut o = opts();
        o.media_type = Some("bmp".to_string());
        assert!(build_from_stdin(ArtifactKind::Image, b"x".as_slice(), &o).is_err());
    }
}
