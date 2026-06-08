/// Serialise a conversation to a Markdown transcript, for "copy as Markdown"
/// (a common chat-client export). Pure, so the format is unit-tested without a
/// clipboard or the DOM.
import type { Message } from "$lib/stores/conversation";

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
