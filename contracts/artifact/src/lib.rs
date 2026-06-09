//! The shared typed artifact envelope (terminal.md §5).
//!
//! An artifact is a rich object a program, the agent or the backend hands to the
//! terminal/harness for display: a typed payload (markdown, code, a table, a chart,
//! an image, a diagram, links), a MANDATORY plain-text floor (what a dumb terminal,
//! a `grep` or a copy sees), and backend-stamped metadata.
//!
//! The crate's whole job is the trust spine (terminal.md §6, the iTerm2
//! CVE-2026-41253 fix): the set of kinds is CLOSED (an unknown kind cannot be
//! represented, so it cannot be rendered), the payload shape is bound to the kind
//! at the type level (a `table` kind cannot carry an `image` payload), and an
//! artifact's `origin` is stamped by the RECEIVING side from the channel the bytes
//! arrived on, NEVER read from or trusted from the producer. A program-emitted
//! artifact is therefore always [`ArtifactOrigin::ExternalContent`], and a program
//! may only emit an inert kind (never a `widget`, the agent/composer's trust class).
//!
//! This crate is the contract; the `arlen-artifact` helper (the OSC-sidecar
//! encoder) and the terminal engine (the decoder) both validate through it, so
//! there is one definition of "a valid artifact", not three.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod build;
pub mod osc;

// ---------------------------------------------------------------------------
// Kind
// ---------------------------------------------------------------------------

/// The closed set of artifact kinds. There is no open MIME string: an unknown kind
/// cannot be represented, so it cannot be stored or rendered (terminal.md §5).
/// Deserialising an unknown tag fails - the validation floor, not a silent default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Markdown prose. Inert: safe from any origin.
    Markdown,
    /// A source-code block with an optional language hint in the payload.
    Code,
    /// Tabular data (header + rows). Inert.
    Table,
    /// A chart specification (typed series, never executable). Inert.
    Chart,
    /// A raster or vector image, rendered as a pixel-inert textured quad. Inert.
    Image,
    /// A diagram source (e.g. a graph description). Inert.
    Diagram,
    /// A list of links, each rendered through the scheme allow-list. Inert.
    Links,
    /// A composer/agent widget. A SEPARATE trust class: only the agent or composer
    /// may mint one. The `arlen-artifact` helper refuses to emit this kind - a
    /// program-emitted widget is the forbidden diagonal.
    Widget,
}

impl ArtifactKind {
    /// Whether this kind is inert (harmless from any origin). The seven inert kinds
    /// are origin-invariant; `Widget` is not inert.
    pub fn is_inert(self) -> bool {
        !matches!(self, ArtifactKind::Widget)
    }

    /// Whether a program (a non-agent producer over the OSC sidecar) is permitted to
    /// emit this kind. Equivalent to `is_inert` today, but a distinct concept: it is
    /// the producer-authority gate, not the render-trust gate.
    pub fn program_emittable(self) -> bool {
        self.is_inert()
    }

    /// The wire/CLI string for this kind (matches the serde `snake_case` tag). Used
    /// by the helper's `--kind` arg parsing and error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            ArtifactKind::Markdown => "markdown",
            ArtifactKind::Code => "code",
            ArtifactKind::Table => "table",
            ArtifactKind::Chart => "chart",
            ArtifactKind::Image => "image",
            ArtifactKind::Diagram => "diagram",
            ArtifactKind::Links => "links",
            ArtifactKind::Widget => "widget",
        }
    }
}

impl std::str::FromStr for ArtifactKind {
    type Err = ArtifactError;

    /// Parse a kind from its wire/CLI string. An unknown string is a clean
    /// rejection (a better error for the CLI than a serde untagged failure); the
    /// strings match the serde `snake_case` tags so wire and CLI agree.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "markdown" => Ok(ArtifactKind::Markdown),
            "code" => Ok(ArtifactKind::Code),
            "table" => Ok(ArtifactKind::Table),
            "chart" => Ok(ArtifactKind::Chart),
            "image" => Ok(ArtifactKind::Image),
            "diagram" => Ok(ArtifactKind::Diagram),
            "links" => Ok(ArtifactKind::Links),
            "widget" => Ok(ArtifactKind::Widget),
            other => Err(ArtifactError::Malformed(format!("unknown kind: {other}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// Origin
// ---------------------------------------------------------------------------

/// Where an artifact's bytes came from, stamped by the backend from the channel
/// they arrived on - NEVER read from the payload (terminal.md §5, the iTerm2
/// CVE-2026-41253 fix; §6 trust spine: "origin is backend-stamped, never inferred
/// from the stream").
///
/// Mirrors the tiering of `ai-core`'s prompt `Origin` without depending on the AI
/// layer: a program-emitted artifact is always `ExternalContent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOrigin {
    /// Output from an external program over the OSC sidecar / a pipe. The
    /// highest-risk origin; untrusted; the only origin a program can carry.
    ExternalContent,
    /// Produced by the contained agent through the os-sdk builder. Untrusted chrome
    /// still routes through the inert path; reserved for the harness.
    AgentGenerated,
    /// Minted by the backend's trust spine (the sole minter of interactive or trust
    /// chrome). Not constructible from this crate's program path.
    SystemTrusted,
}

// ---------------------------------------------------------------------------
// Payload supporting types
// ---------------------------------------------------------------------------

/// A chart type. A closed enum so a chart spec can never carry an arbitrary
/// renderer directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    /// A line chart.
    Line,
    /// A bar chart.
    Bar,
    /// A scatter plot.
    Scatter,
    /// A pie chart.
    Pie,
    /// An area chart.
    Area,
}

/// One labelled numeric series of a chart. The values are plain numbers, never an
/// executable expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// The series label.
    pub name: String,
    /// The numeric data points.
    pub values: Vec<f64>,
}

/// The media type of an image payload. Closed so an arbitrary MIME string cannot
/// ride in. `Svg` is a vector format and MUST be sanitised at render time (it can
/// carry script); that sanitisation is the renderer's job, not this crate's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageMediaType {
    /// `image/png`.
    Png,
    /// `image/jpeg`.
    Jpeg,
    /// `image/svg+xml` - sanitise at render (may carry script).
    Svg,
    /// `image/webp`.
    Webp,
    /// `image/gif`.
    Gif,
}

/// A diagram source language. Closed; the renderer maps it to a sandboxed diagram
/// engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramLanguage {
    /// Mermaid diagram source.
    Mermaid,
    /// Graphviz DOT source.
    Dot,
}

/// A single link. The scheme is NOT validated here - the renderer applies its
/// scheme allow-list (terminal.md §365); this crate only types the shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// The link target.
    pub href: String,
    /// An optional display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

// ---------------------------------------------------------------------------
// Payload
// ---------------------------------------------------------------------------

/// The typed body of an artifact, one variant per inert kind. The variant MUST
/// agree with the envelope's `kind` (checked in [`Artifact::new`]). `Widget` has no
/// program-constructible payload variant here - it is minted by the harness
/// builder, not this crate's program path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactPayload {
    /// `markdown`: the markdown source.
    Markdown {
        /// The markdown source.
        source: String,
    },
    /// `code`: the source plus an optional language hint.
    Code {
        /// The source code.
        source: String,
        /// An optional language hint (e.g. `"rust"`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    /// `table`: a header row and the data rows (all cells are strings; the renderer
    /// never evaluates a cell).
    Table {
        /// The header row.
        columns: Vec<String>,
        /// The data rows; each is a list of string cells.
        rows: Vec<Vec<String>>,
    },
    /// `chart`: a typed chart spec - a chart type plus labelled numeric series.
    /// Never an executable expression.
    Chart {
        /// The chart type.
        chart_type: ChartType,
        /// The labelled numeric series.
        series: Vec<Series>,
    },
    /// `image`: base64 image bytes plus the image media type. Rendered as a
    /// pixel-inert textured quad; the media type is from a closed enum.
    Image {
        /// The image media type.
        media_type: ImageMediaType,
        /// The base64-encoded image bytes.
        data_base64: String,
    },
    /// `diagram`: a diagram-language tag plus its source.
    Diagram {
        /// The diagram language.
        language: DiagramLanguage,
        /// The diagram source.
        source: String,
    },
    /// `links`: a list of typed links, each rendered through the scheme allow-list.
    Links {
        /// The links.
        links: Vec<Link>,
    },
}

impl ArtifactPayload {
    /// The kind this payload variant corresponds to. Used by [`Artifact::new`] to
    /// verify the declared kind matches the payload.
    pub fn kind(&self) -> ArtifactKind {
        match self {
            ArtifactPayload::Markdown { .. } => ArtifactKind::Markdown,
            ArtifactPayload::Code { .. } => ArtifactKind::Code,
            ArtifactPayload::Table { .. } => ArtifactKind::Table,
            ArtifactPayload::Chart { .. } => ArtifactKind::Chart,
            ArtifactPayload::Image { .. } => ArtifactKind::Image,
            ArtifactPayload::Diagram { .. } => ArtifactKind::Diagram,
            ArtifactPayload::Links { .. } => ArtifactKind::Links,
        }
    }
}

// ---------------------------------------------------------------------------
// Metadata + envelope
// ---------------------------------------------------------------------------

/// Backend-stamped metadata. The `origin` here is authoritative and is set by the
/// receiving side from the channel the bytes arrived on; it is never deserialised
/// from an untrusted producer's bytes (see [`Artifact::receive`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactMeta {
    /// The provenance tier. Backend-stamped, never payload-derived.
    pub origin: ArtifactOrigin,
    /// An optional human label for the artifact (e.g. a title shown in a
    /// pinned-artifact list). Cosmetic; carries no authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// A unified rich-object artifact: a typed payload, a MANDATORY plain-text floor,
/// and backend-stamped metadata (terminal.md §5). Constructed only through
/// `new`/`receive`, which enforce the two invariants: `text` is non-empty and
/// `kind` agrees with the payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    /// The closed kind. Redundant with `payload.kind()` on the wire, but present so
    /// a reader can branch on kind without matching the payload.
    pub kind: ArtifactKind,
    /// The typed body.
    pub payload: ArtifactPayload,
    /// The MANDATORY plain-text fallback - the pipe floor. A dumb terminal, a
    /// `grep`, a copy all see this. Must be non-empty.
    pub text: String,
    /// Backend-stamped metadata, including the authoritative origin.
    pub meta: ArtifactMeta,
}

/// Why an artifact failed to construct or deserialise.
#[derive(Debug, Error, PartialEq)]
pub enum ArtifactError {
    /// `text` was empty - the mandatory plain-text floor is missing.
    #[error("artifact text is mandatory and must be non-empty")]
    MissingText,
    /// The declared `kind` did not match the payload variant.
    #[error("kind {declared:?} does not match payload kind {payload:?}")]
    KindMismatch {
        /// The kind the envelope declared.
        declared: ArtifactKind,
        /// The kind the payload actually is.
        payload: ArtifactKind,
    },
    /// A program tried to emit a kind it is not authorised to produce (today:
    /// `widget`, and any non-inert kind).
    #[error("kind {0:?} is not program-emittable")]
    NotProgramEmittable(ArtifactKind),
    /// The wire bytes did not parse as a valid envelope (unknown kind, malformed
    /// payload, bad JSON).
    #[error("malformed artifact: {0}")]
    Malformed(String),
    /// A chart series carried a non-finite value (NaN or infinity). `serde_json`
    /// cannot serialise a non-finite `f64`, so accepting one would make a later
    /// `to_json` panic; it is rejected at construction instead.
    #[error("chart series contains a non-finite value")]
    NonFiniteChartValue,
}

/// Reject a payload that could not be serialised back to JSON. The only such case
/// is a chart series with a non-finite `f64` (`serde_json` errors on NaN/infinity),
/// which would otherwise turn a later `to_json` into a panic. Validated at every
/// construction entry point so `to_json` is genuinely infallible for a constructed
/// artifact.
fn validate_payload(payload: &ArtifactPayload) -> Result<(), ArtifactError> {
    if let ArtifactPayload::Chart { series, .. } = payload {
        for s in series {
            if s.values.iter().any(|v| !v.is_finite()) {
                return Err(ArtifactError::NonFiniteChartValue);
            }
        }
    }
    Ok(())
}

/// A program's envelope: payload + text (+ optional title) only. Deliberately has
/// NO `origin`/`kind` field, so a forged origin or kind in the incoming bytes is
/// dropped by serde before it can reach the validator (the channel-authority
/// boundary). Extra fields are ignored (serde default), so a producer cannot smuggle
/// authority in through an unknown key either.
#[derive(Deserialize)]
struct ProgramEnvelope {
    payload: ArtifactPayload,
    text: String,
    #[serde(default)]
    title: Option<String>,
}

impl Artifact {
    /// Construct and validate an artifact with an explicit, caller-supplied origin.
    /// Enforces: `text` non-empty, `kind == payload.kind()`. The caller is asserting
    /// authority for `origin` - this is the in-process builder path (the
    /// agent/backend). Programs MUST NOT reach this with a trusted origin; they go
    /// through [`Artifact::receive`].
    pub fn new(
        payload: ArtifactPayload,
        text: String,
        origin: ArtifactOrigin,
        title: Option<String>,
    ) -> Result<Self, ArtifactError> {
        if text.is_empty() {
            return Err(ArtifactError::MissingText);
        }
        validate_payload(&payload)?;
        let kind = payload.kind();
        Ok(Artifact {
            kind,
            payload,
            text,
            meta: ArtifactMeta { origin, title },
        })
    }

    /// Construct an artifact from an external program's payload + text, stamping
    /// [`ArtifactOrigin::ExternalContent`] UNCONDITIONALLY. This is the
    /// channel-authority boundary: the origin is set by the receiving side, never
    /// read from or trusted from the producer (terminal.md §5/§6, the iTerm2
    /// CVE-2026-41253 fix). Additionally rejects any kind that is not
    /// `program_emittable` (no `widget`). This is the function the OSC-sidecar
    /// decoder and the `arlen-artifact` helper both call.
    pub fn receive(
        payload: ArtifactPayload,
        text: String,
        title: Option<String>,
    ) -> Result<Self, ArtifactError> {
        if !payload.kind().program_emittable() {
            return Err(ArtifactError::NotProgramEmittable(payload.kind()));
        }
        // origin is ALWAYS ExternalContent here - the load-bearing line: a
        // program-emitted artifact is never trusted, the origin is set by the
        // receiving side, never read from the payload.
        Self::new(payload, text, ArtifactOrigin::ExternalContent, title)
    }

    /// Parse a JSON envelope and RE-VALIDATE it (unknown kind / malformed payload /
    /// missing text / kind-payload mismatch all reject). Used by a consumer reading
    /// a full envelope off the wire; serde alone validates shape, not the cross-field
    /// invariants, so `new`'s checks are re-run. This trusts the `origin` carried in
    /// the bytes - it is for backend-internal envelopes, NOT a program channel. A
    /// program's bytes go through [`Artifact::receive_json`], which discards origin.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ArtifactError> {
        let raw: Artifact = serde_json::from_slice(bytes)
            .map_err(|e| ArtifactError::Malformed(e.to_string()))?;
        // Re-run the cross-field invariants serde does not check.
        if raw.text.is_empty() {
            return Err(ArtifactError::MissingText);
        }
        let payload_kind = raw.payload.kind();
        if raw.kind != payload_kind {
            return Err(ArtifactError::KindMismatch {
                declared: raw.kind,
                payload: payload_kind,
            });
        }
        validate_payload(&raw.payload)?;
        Ok(raw)
    }

    /// Parse a program's JSON envelope (payload + text + optional title only) and
    /// stamp [`ArtifactOrigin::ExternalContent`]. Any `origin`/`kind`/`meta` present
    /// in the incoming bytes is DISCARDED (the [`ProgramEnvelope`] shape has no such
    /// field, so serde never reads it) and overwritten. This is the decoder side of
    /// the OSC sidecar.
    pub fn receive_json(bytes: &[u8]) -> Result<Self, ArtifactError> {
        let env: ProgramEnvelope = serde_json::from_slice(bytes)
            .map_err(|e| ArtifactError::Malformed(e.to_string()))?;
        Self::receive(env.payload, env.text, env.title)
    }

    /// Serialise to the canonical JSON envelope.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("artifact serialization is infallible for owned data")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn table_payload() -> ArtifactPayload {
        ArtifactPayload::Table {
            columns: vec!["a".into(), "b".into()],
            rows: vec![vec!["1".into(), "2".into()]],
        }
    }

    #[test]
    fn text_is_mandatory() {
        let err = Artifact::new(
            table_payload(),
            String::new(),
            ArtifactOrigin::SystemTrusted,
            None,
        )
        .unwrap_err();
        assert_eq!(err, ArtifactError::MissingText);
    }

    #[test]
    fn unknown_kind_rejected_on_wire() {
        let bytes = br#"{"kind":"hologram","payload":{"kind":"markdown","source":"x"},"text":"t","meta":{"origin":"system_trusted"}}"#;
        assert!(matches!(
            Artifact::from_json(bytes),
            Err(ArtifactError::Malformed(_))
        ));
        assert!(ArtifactKind::from_str("hologram").is_err());
    }

    #[test]
    fn kind_payload_mismatch_rejected() {
        // Top-level kind "table" but a markdown payload.
        let bytes = br#"{"kind":"table","payload":{"kind":"markdown","source":"x"},"text":"t","meta":{"origin":"system_trusted"}}"#;
        assert_eq!(
            Artifact::from_json(bytes),
            Err(ArtifactError::KindMismatch {
                declared: ArtifactKind::Table,
                payload: ArtifactKind::Markdown,
            })
        );
    }

    #[test]
    fn widget_not_program_emittable() {
        assert!(!ArtifactKind::Widget.program_emittable());
        assert!(!ArtifactKind::Widget.is_inert());
        // There is no Widget payload variant, so a program can never construct one;
        // the seven payload variants are all inert/emittable.
        for k in [
            ArtifactKind::Markdown,
            ArtifactKind::Code,
            ArtifactKind::Table,
            ArtifactKind::Chart,
            ArtifactKind::Image,
            ArtifactKind::Diagram,
            ArtifactKind::Links,
        ] {
            assert!(k.program_emittable(), "{k:?} must be program-emittable");
        }
    }

    #[test]
    fn receive_stamps_external_content() {
        // Even when the incoming JSON carries a forged trusted origin, receive_json
        // discards it and stamps ExternalContent. This is the CVE-fix test.
        let bytes = br#"{"payload":{"kind":"markdown","source":"hi"},"text":"hi","origin":"system_trusted","meta":{"origin":"system_trusted"}}"#;
        let art = Artifact::receive_json(bytes).unwrap();
        assert_eq!(
            art.meta.origin,
            ArtifactOrigin::ExternalContent,
            "a program's forged origin must be discarded"
        );
        // And the direct builder path stamps ExternalContent too.
        let art2 = Artifact::receive(
            ArtifactPayload::Markdown { source: "x".into() },
            "x".into(),
            None,
        )
        .unwrap();
        assert_eq!(art2.meta.origin, ArtifactOrigin::ExternalContent);
    }

    #[test]
    fn receive_refuses_a_program_widget() {
        // A widget has no payload variant, but the program-emittable gate is the
        // belt: any future kind that is not inert is refused at receive.
        // Construct via from_json is impossible (no Widget payload), so assert the
        // gate directly on the kind.
        assert!(
            !ArtifactKind::Widget.program_emittable(),
            "widget is the forbidden diagonal for a program"
        );
    }

    #[test]
    fn roundtrip() {
        let art = Artifact::new(
            table_payload(),
            "a\tb\n1\t2".into(),
            ArtifactOrigin::AgentGenerated,
            Some("My table".into()),
        )
        .unwrap();
        let bytes = art.to_json();
        let back = Artifact::from_json(&bytes).unwrap();
        assert_eq!(art, back);
    }

    #[test]
    fn inert_kinds_exact() {
        // The inert set is exactly the seven named kinds; Widget is the only
        // non-inert. Guards against someone adding a kind and mis-marking it inert.
        let inert: Vec<ArtifactKind> = [
            ArtifactKind::Markdown,
            ArtifactKind::Code,
            ArtifactKind::Table,
            ArtifactKind::Chart,
            ArtifactKind::Image,
            ArtifactKind::Diagram,
            ArtifactKind::Links,
            ArtifactKind::Widget,
        ]
        .into_iter()
        .filter(|k| k.is_inert())
        .collect();
        assert_eq!(inert.len(), 7);
        assert!(!inert.contains(&ArtifactKind::Widget));
    }

    #[test]
    fn non_finite_chart_value_is_rejected() {
        // serde_json cannot serialise NaN/infinity, so a chart carrying one would
        // make to_json panic; it must be rejected at construction instead.
        let nan_chart = ArtifactPayload::Chart {
            chart_type: ChartType::Line,
            series: vec![Series {
                name: "x".into(),
                values: vec![1.0, f64::NAN],
            }],
        };
        assert_eq!(
            Artifact::new(nan_chart.clone(), "t".into(), ArtifactOrigin::AgentGenerated, None),
            Err(ArtifactError::NonFiniteChartValue)
        );
        assert_eq!(
            Artifact::receive(nan_chart, "t".into(), None),
            Err(ArtifactError::NonFiniteChartValue)
        );
        // A finite chart constructs and to_json does not panic.
        let ok_chart = ArtifactPayload::Chart {
            chart_type: ChartType::Bar,
            series: vec![Series { name: "y".into(), values: vec![1.0, 2.5] }],
        };
        let art = Artifact::new(ok_chart, "t".into(), ArtifactOrigin::AgentGenerated, None).unwrap();
        let _ = art.to_json();
    }

    #[test]
    fn as_str_round_trips_through_from_str() {
        for k in [
            ArtifactKind::Markdown,
            ArtifactKind::Code,
            ArtifactKind::Table,
            ArtifactKind::Chart,
            ArtifactKind::Image,
            ArtifactKind::Diagram,
            ArtifactKind::Links,
            ArtifactKind::Widget,
        ] {
            assert_eq!(ArtifactKind::from_str(k.as_str()).unwrap(), k);
        }
    }
}
