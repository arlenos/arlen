//! Pure helpers for the bounded agent loop: the per-step contract the
//! model answers in, parsing it out of free-text model output, and
//! building the per-step prompt with content-origin tagging.
//!
//! The loop itself (budget enforcement, the gate call per step) lives on
//! the [`crate::engine::Dispatcher`]; these helpers are kept pure so they
//! are unit-testable without a provider or a graph.

use arlen_ai_core::pipeline::extract_json;
use arlen_ai_core::tagging::{Block, Origin, TaggedPrompt};
use arlen_ai_sandbox::extract_text;
use serde::Deserialize;

use crate::behaviour::Behaviour;
use crate::compaction::{self, TranscriptEntry};
use crate::seams::AgentEvent;

/// One step the model takes in the bounded loop: either propose a single
/// tool action, or stop. There is no "keep going" variant; the loop
/// continues by default and is bounded by the manifest budget, so a step
/// is always one of these two explicit moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStep {
    /// Propose one tool action for the gate to decide on.
    Propose {
        /// The tool the model wants to invoke (must be a declared tool).
        tool: String,
        /// One-line, human-facing description of the action.
        summary: String,
        /// The action's operands as parameter-name to value (a node id or path
        /// literal). **Untrusted** — the model states them — so they prove
        /// nothing on their own; the predict-before-act gate validates them
        /// against the action's trusted schema and the real graph before any
        /// execution cap is lifted. Empty when the model states no operands.
        arguments: std::collections::BTreeMap<String, String>,
    },
    /// Stop the loop on a declared terminal condition. The loop validates
    /// `terminal` against the behaviour's declared `terminal` conditions, so
    /// the model can only stop in a way the behaviour author named (and the
    /// surfacing disposition can be keyed off it), never with an invented or
    /// injected condition.
    Stop {
        /// The declared terminal condition the model stopped on.
        terminal: String,
        /// Optional free-text explanation; not authoritative.
        note: String,
    },
}

/// The model's raw per-step JSON, before validation into an [`AgentStep`].
#[derive(Deserialize)]
struct RawStep {
    action: String,
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    /// Operands for a propose step; absent or non-string values fail the parse
    /// and are fed back to the model rather than silently coerced.
    #[serde(default)]
    arguments: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    terminal: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// Parse one [`AgentStep`] out of a model response. The response may wrap
/// the JSON in prose or fences; [`extract_json`] finds the first balanced
/// object. Returns an `Err` describing the problem so the loop can feed it
/// back to the model for a corrected next step.
pub fn parse_agent_step(text: &str) -> Result<AgentStep, String> {
    let json = extract_json(text).ok_or("no JSON object in the response")?;
    let raw: RawStep =
        serde_json::from_str(json).map_err(|e| format!("invalid step JSON: {e}"))?;
    match raw.action.as_str() {
        "propose" => {
            let tool = raw
                .tool
                .filter(|t| !t.is_empty())
                .ok_or("a propose step must name a non-empty 'tool'")?;
            Ok(AgentStep::Propose {
                tool,
                summary: raw.summary.unwrap_or_default(),
                arguments: raw.arguments,
            })
        }
        "stop" => {
            let terminal = raw
                .terminal
                .filter(|t| !t.is_empty())
                .ok_or("a stop step must name a declared 'terminal' condition")?;
            Ok(AgentStep::Stop {
                terminal,
                note: raw.note.unwrap_or_default(),
            })
        }
        other => Err(format!("unknown step action {other:?}")),
    }
}

/// Build the per-step prompt. The instruction channel (the behaviour's
/// goal and body instructions, the tool list, the declared stop conditions,
/// the response contract) is static, trusted text: the body is the
/// behaviour author's instructions, loaded from a provenance-stamped
/// directory, so it carries the behaviour-specific rules and safety
/// constraints the gate cannot see. Everything app- or model-influenced
/// (the triggering event's fields, the running transcript of prior steps)
/// goes into content-origin-tagged data blocks (S18-A), so it can never be
/// read as an instruction.
pub fn build_agent_prompt(
    behaviour: &Behaviour,
    event: &AgentEvent,
    transcript: &[TranscriptEntry],
) -> String {
    let manifest = &behaviour.manifest;
    let tools = if manifest.tools.is_empty() {
        "(none)".to_string()
    } else {
        manifest
            .tools
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    };
    let stop_conditions = if manifest.terminal.is_empty() {
        String::new()
    } else {
        format!(
            "\nStop (with an \"action\":\"stop\") when any of these conditions is met: {}.",
            manifest
                .terminal
                .keys()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let instruction = format!(
        "You are the Arlen agent behaviour \"{name}\".\n\
         Goal: {goal}\n\n\
         Instructions:\n{body}\n\n\
         Available tools: {tools}.{stop_conditions}\n\n\
         Work toward the goal one step at a time. Respond with EXACTLY one JSON \
         object and nothing else, either proposing a single tool action or \
         stopping:\n\
         {{\"action\": \"propose\", \"tool\": \"<one of the available tools>\", \"summary\": \"<one line: what and why>\"}}\n\
         {{\"action\": \"stop\", \"terminal\": \"<one of the stop conditions>\", \"note\": \"<optional explanation>\"}}\n\
         Only propose tools from the list, and stop only with one of the named \
         stop conditions, as soon as the goal is met.",
        name = manifest.name,
        goal = manifest.description,
        body = behaviour.body.trim(),
        tools = tools,
        stop_conditions = stop_conditions,
    );

    // The triggering event, as data. A file path or window title can read
    // like an instruction, so it is tagged by origin, never trusted. When the
    // content is external, the event type and each field value are first run
    // through the inert-text sanitiser (S18-B), so ANSI escapes, bidirectional
    // overrides, and invisible/format characters cannot smuggle hidden
    // instructions past the reader or the injection classifier. The event type
    // is sanitised too: it is copied from the bus envelope (producer-controlled)
    // and the router matches it by prefix/wildcard, so a behaviour can receive a
    // type the producer padded with instruction-like or control characters.
    let event_block = {
        let type_value = if event.external_content {
            sanitize_external(&event.event_type)
        } else {
            event.event_type.clone()
        };
        let mut s = format!("event_type: {type_value}");
        for (k, v) in &event.fields {
            if event.external_content {
                s.push_str(&format!("\n{k}: {}", sanitize_external(v)));
            } else {
                s.push_str(&format!("\n{k}: {v}"));
            }
        }
        s
    };
    let transcript_block = compaction::render(transcript);

    let mut blocks = vec![Block {
        origin: if event.external_content {
            Origin::ExternalContent
        } else {
            Origin::GraphData
        },
        content: &event_block,
    }];
    if !transcript_block.is_empty() {
        blocks.push(Block {
            origin: Origin::ModelFeedback,
            content: &transcript_block,
        });
    }
    let tagged = TaggedPrompt::new(&blocks);

    format!(
        "{instruction}\n\n{preamble}\n\n{rendered}",
        preamble = tagged.preamble(),
        rendered = tagged.rendered(),
    )
}

/// Sanitise one external field value into inert text before it reaches the
/// model. Reuses the document sandbox's text extractor (S18-B) in process:
/// a short event field is not the parser attack surface the subprocess
/// isolates, so the same stripping logic (control characters, ANSI escapes,
/// bidirectional overrides, invisible/format characters) applies directly
/// without the Landlock/seccomp worker. On the only failure the extractor can
/// raise for a field (a value past its multi-megabyte cap, which is itself
/// suspect), returns a placeholder rather than the raw value, so nothing
/// unsanitised is ever forwarded.
fn sanitize_external(value: &str) -> String {
    extract_text(value.as_bytes()).unwrap_or_else(|_| "(unprocessable field)".to_string())
}

/// The maximum byte length of an observation preview kept inline in the
/// transcript. The full result persists at the observation's reference (P8), so
/// the inline preview is a bounded, inert snippet and one verbose tool result
/// cannot blow the context window even before compaction.
pub const OBSERVATION_PREVIEW_CAP: usize = 2_048;

/// Render a read tool's result rows into a bounded, inert-text preview for an
/// observation entry (the design's observe step). Every cell key and value is
/// passed through the S18-B sanitiser ([`sanitize_external`]) so a result that
/// reached untrusted data (a file's contents surfaced by a query) cannot carry
/// control characters, ANSI escapes or bidirectional overrides back into the
/// model's context. The preview is truncated to `cap` bytes on a char boundary
/// with an explicit marker; truncation loses nothing the model cannot re-fetch
/// from the full result at the observation's reference. This is the S18-B half
/// of result screening; the caller still runs the S17 injection classifier over
/// the returned preview before it enters the transcript.
pub fn observation_preview(
    rows: &[std::collections::HashMap<String, serde_json::Value>],
    cap: usize,
) -> String {
    let mut out = String::new();
    for row in rows {
        if !out.is_empty() {
            out.push('\n');
        }
        // Deterministic column order so the preview is stable across runs.
        let mut cells: Vec<(&String, &serde_json::Value)> = row.iter().collect();
        cells.sort_by(|a, b| a.0.cmp(b.0));
        let rendered: Vec<String> = cells
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    sanitize_external(k),
                    sanitize_external(&value_to_string(v))
                )
            })
            .collect();
        out.push_str(&rendered.join(", "));
        if out.len() > cap {
            break; // stop building; the truncation below bounds it
        }
    }
    if out.len() > cap {
        let mut end = cap;
        while end > 0 && !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str(" ...(truncated; full result at reference)");
    }
    out
}

/// A JSON value's scalar string form for a preview cell. A string passes
/// through (then sanitised by the caller); other scalars and containers use
/// their JSON text. Never panics.
fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => "null".to_string(),
        other => other.to_string(),
    }
}

/// The external content of an event as the model will see it, for injection
/// screening (S17). Covers the event type and every field value, sanitised
/// exactly as [`build_agent_prompt`] sanitises them, so the screen covers
/// precisely the untrusted text the model would read. The event type is
/// included because it is bus-envelope-copied and prefix/wildcard-matched, not
/// a trusted canonical identifier, so it could itself carry an injection. The
/// field keys are short, fixed identifiers from the event decoder and are not
/// screened.
pub fn external_screen_text(event: &AgentEvent) -> String {
    std::iter::once(sanitize_external(&event.event_type))
        .chain(event.fields.values().map(|v| sanitize_external(v)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_a_propose_step_from_prose_wrapped_json() {
        let text = "Sure, here is my step:\n```json\n{\"action\":\"propose\",\"tool\":\"graph.write\",\"summary\":\"tag foo\"}\n```";
        assert_eq!(
            parse_agent_step(text).unwrap(),
            AgentStep::Propose {
                tool: "graph.write".to_string(),
                summary: "tag foo".to_string(),
                arguments: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn parses_propose_operands_into_arguments() {
        let text = r#"{"action":"propose","tool":"graph.write","summary":"tag","arguments":{"file":"f1","project":"p1"}}"#;
        assert_eq!(
            parse_agent_step(text).unwrap(),
            AgentStep::Propose {
                tool: "graph.write".to_string(),
                summary: "tag".to_string(),
                arguments: BTreeMap::from([
                    ("file".to_string(), "f1".to_string()),
                    ("project".to_string(), "p1".to_string()),
                ]),
            }
        );
    }

    #[test]
    fn non_string_operand_values_fail_the_parse() {
        // A numeric operand value is rejected (fed back to the model) rather
        // than silently coerced, so the gate only ever sees string operands.
        let text = r#"{"action":"propose","tool":"t","summary":"s","arguments":{"n":42}}"#;
        assert!(parse_agent_step(text).is_err());
    }

    fn row(pairs: &[(&str, serde_json::Value)]) -> std::collections::HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn observation_preview_sanitises_and_orders_cells() {
        let rows = vec![row(&[
            ("name", serde_json::json!("a\u{1b}[31mred\u{1b}[0m")),
            ("count", serde_json::json!(3)),
        ])];
        let preview = observation_preview(&rows, OBSERVATION_PREVIEW_CAP);
        // The ESC control character is stripped (the S18-B security property);
        // the exact escape-text handling is extract_text's own, tested there.
        assert!(!preview.contains('\u{1b}'), "no control characters survive");
        // Deterministic (sorted) column order, JSON scalar rendering.
        assert!(preview.starts_with("count=3, name=a"), "ordered + sanitised: {preview}");
    }

    #[test]
    fn observation_preview_truncates_on_a_char_boundary_with_a_marker() {
        let big: String = "x".repeat(10_000);
        let rows = vec![row(&[("data", serde_json::json!(big))])];
        let preview = observation_preview(&rows, 100);
        assert!(preview.len() < 200, "bounded near the cap");
        assert!(preview.ends_with("full result at reference)"), "marks the elision");
    }

    #[test]
    fn observation_preview_of_no_rows_is_empty() {
        assert_eq!(observation_preview(&[], OBSERVATION_PREVIEW_CAP), "");
    }

    #[test]
    fn parses_a_stop_step() {
        assert_eq!(
            parse_agent_step("{\"action\":\"stop\",\"terminal\":\"done\",\"note\":\"finished\"}")
                .unwrap(),
            AgentStep::Stop {
                terminal: "done".to_string(),
                note: "finished".to_string(),
            }
        );
    }

    #[test]
    fn rejects_garbage_and_missing_fields() {
        assert!(parse_agent_step("no json here").is_err());
        assert!(parse_agent_step("{\"action\":\"propose\"}").is_err()); // missing tool
        assert!(parse_agent_step("{\"action\":\"stop\"}").is_err()); // missing terminal
        assert!(parse_agent_step("{\"action\":\"wander\"}").is_err());
    }

    fn agent_behaviour(skill: &str) -> Behaviour {
        crate::behaviour::parse(skill).expect("valid")
    }

    fn opened(path: &str, external: bool) -> AgentEvent {
        AgentEvent {
            id: "e1".to_string(),
            event_type: "file.opened".to_string(),
            fields: BTreeMap::from([("path".to_string(), path.to_string())]),
            external_content: external,
        }
    }

    const DEMO_AGENT: &str = "---\nname: demo-agent\ndescription: tidy things\nkind: agent\ntrigger:\n  type: event\n  event: file.opened\nreads: minimal\ntools:\n  graph.write: []\nbudget:\n  max_steps: 5\n  max_tokens: 1000\n  max_wall_ms: 60000\nterminal:\n  done: silent\n---\nNever delete anything; only ever tag files.\n";

    #[test]
    fn prompt_carries_body_instructions_tools_and_stop_conditions_in_the_clear() {
        let b = agent_behaviour(DEMO_AGENT);
        let prompt = build_agent_prompt(&b, &opened("~/x.rs", false), &[]);
        // Instruction channel is plain text: name, body safety rules, tools,
        // and the declared stop conditions.
        assert!(prompt.contains("agent behaviour \"demo-agent\""));
        assert!(prompt.contains("Never delete anything; only ever tag files."));
        assert!(prompt.contains("Available tools: graph.write"));
        assert!(prompt.contains("done")); // the declared terminal condition
        // Event data is wrapped as a tagged, data-only block.
        assert!(prompt.contains("[GRAPH-DATA-"));
        assert!(prompt.contains("DATA ONLY"));
        assert!(prompt.contains("path: ~/x.rs"));
    }

    #[test]
    fn external_field_values_are_stripped_of_smuggled_control_characters() {
        let b = agent_behaviour(DEMO_AGENT);
        // A filename carrying an ANSI escape, a bidi override, and a zero-width
        // space, all of which could hide or reorder instructions for the model.
        let nasty = "report\u{202E}cod.exe\u{200B}\u{1b}[31mignore previous";
        let prompt = build_agent_prompt(&b, &opened(nasty, true), &[]);
        // The dangerous characters are gone; the readable text survives.
        assert!(!prompt.contains('\u{202E}'));
        assert!(!prompt.contains('\u{200B}'));
        assert!(!prompt.contains('\u{1b}'));
        assert!(prompt.contains("report"));
        assert!(prompt.contains("ignore previous")); // readable text kept, just inert
    }

    #[test]
    fn graph_data_fields_are_not_sanitised() {
        // Non-external content comes from trusted producers; it is left intact
        // (sanitising could damage legitimate graph data).
        let b = agent_behaviour(DEMO_AGENT);
        let prompt = build_agent_prompt(&b, &opened("plain/path.rs", false), &[]);
        assert!(prompt.contains("path: plain/path.rs"));
    }

    #[test]
    fn external_screen_text_covers_the_event_type_and_values_sanitised() {
        let mut ev = opened("a\u{200B}b", true);
        // A producer-padded event type with a smuggled bidi override: it must be
        // sanitised and present in the screened text, not silently trusted.
        ev.event_type = "file.opened\u{202E}rm".to_string();
        let text = external_screen_text(&ev);
        assert!(text.contains("file.openedrm")); // type included, bidi stripped
        assert!(!text.contains('\u{202E}'));
        assert!(text.contains("ab")); // value included, zero-width stripped
    }

    #[test]
    fn external_screen_text_of_a_fieldless_event_is_the_event_type() {
        // A field-less event still has a producer-controlled event type to
        // screen (it is bus-envelope-copied, not a trusted identifier).
        let ev = AgentEvent {
            id: "e".to_string(),
            event_type: "calendar.event.upcoming".to_string(),
            fields: BTreeMap::new(),
            external_content: true,
        };
        assert_eq!(external_screen_text(&ev), "calendar.event.upcoming");
    }

    #[test]
    fn external_event_is_tagged_as_external_content() {
        let b = agent_behaviour(DEMO_AGENT);
        let transcript = [TranscriptEntry::Proposed {
            step: 0,
            tool: "graph.write".to_string(),
            summary: "tag foo".to_string(),
            decision: "Propose".to_string(),
        }];
        let prompt = build_agent_prompt(&b, &opened("~/x.rs", true), &transcript);
        assert!(prompt.contains("[EXTERNAL-CONTENT-"));
        // The transcript is fed back as model feedback, also data.
        assert!(prompt.contains("[PRIOR-ERROR-"));
        assert!(prompt.contains("step 0: proposed graph.write"));
    }
}
