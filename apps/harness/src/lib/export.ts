/// Serialise a conversation to a Markdown transcript, for "copy as Markdown"
/// (a common chat-client export), and to a portable JSON envelope for backup or
/// transfer. Pure, so the formats are unit-tested without a clipboard or the DOM.
import type { Message, Session } from "$lib/stores/conversation";

/// The tag and version of the JSON export envelope, so a future format change is
/// detected on import rather than mis-read.
const EXPORT_FORMAT = "arlen-harness-conversation";
const EXPORT_VERSION = 1;

/// Serialise a whole conversation to a portable JSON envelope (backup or
/// transfer), tagged with a format marker and version so import can reject a
/// foreign or future file. The session is embedded verbatim; import re-validates
/// it through `sanitizeSession`, so a hand-edited file cannot inject a malformed
/// record.
export function conversationToJson(session: Session): string {
  return JSON.stringify(
    { format: EXPORT_FORMAT, version: EXPORT_VERSION, session },
    null,
    2,
  );
}

/// Parse a JSON export envelope, returning the embedded session payload (still
/// untrusted, to be sanitised by the caller) or `null` when the input is not a
/// well-formed envelope of the expected format and version. Deliberately does
/// not sanitise here: that keeps this module free of a value-dependency on the
/// store, and the caller runs `sanitizeSession` as the single validation point.
export function parseConversationEnvelope(json: string): unknown {
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const env = parsed as Record<string, unknown>;
  if (env.format !== EXPORT_FORMAT || env.version !== EXPORT_VERSION) return null;
  return env.session ?? null;
}

/// Render one message as a labelled Markdown block. Pending turns carry no
/// content and are dropped by the caller; an error turn is shown as such so a
/// failed exchange is not silently exported as an answer.
/// Whether a user turn carried file attachments.
function hasAttachments(msg: Message): boolean {
  return msg.role === "user" && !!msg.mentions && msg.mentions.length > 0;
}

function blockFor(msg: Message): string {
  const label = msg.role === "user" ? "You" : msg.role === "assistant" ? "Assistant" : "Error";
  const lines = [`**${label}:**`, ""];
  const text = msg.text.trim();
  if (text.length > 0) lines.push(text);
  if (hasAttachments(msg)) {
    // Separate the note from the text only when there was text.
    if (text.length > 0) lines.push("");
    lines.push(`_Attached: ${msg.mentions!.join(", ")}_`);
  }
  return lines.join("\n");
}

/// Turn a conversation's messages into a Markdown transcript. Pending
/// placeholders are skipped; blocks are separated by a blank line. A turn is
/// included when it has text OR (for a user turn) attachments, so an
/// attachment-only question (which `send` allows) still appears with its
/// `_Attached:_` note rather than vanishing. An empty (or all-pending)
/// conversation yields an empty string, so the caller can decide not to copy
/// nothing.
export function conversationToMarkdown(messages: Message[]): string {
  return messages
    .filter((m) => !m.pending && (m.text.trim().length > 0 || hasAttachments(m)))
    .map(blockFor)
    .join("\n\n");
}
