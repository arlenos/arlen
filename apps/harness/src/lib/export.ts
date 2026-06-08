/// Serialise a conversation to a Markdown transcript, for "copy as Markdown"
/// (a common chat-client export). Pure, so the format is unit-tested without a
/// clipboard or the DOM.
import type { Message } from "$lib/stores/conversation";

/// Render one message as a labelled Markdown block. Pending turns carry no
/// content and are dropped by the caller; an error turn is shown as such so a
/// failed exchange is not silently exported as an answer.
function blockFor(msg: Message): string {
  const label = msg.role === "user" ? "You" : msg.role === "assistant" ? "Assistant" : "Error";
  const lines = [`**${label}:**`, "", msg.text.trim()];
  if (msg.role === "user" && msg.mentions && msg.mentions.length > 0) {
    lines.push("", `_Attached: ${msg.mentions.join(", ")}_`);
  }
  return lines.join("\n");
}

/// Turn a conversation's messages into a Markdown transcript. Pending
/// placeholders and empty messages are skipped; blocks are separated by a blank
/// line. An empty (or all-pending) conversation yields an empty string, so the
/// caller can decide not to copy nothing.
export function conversationToMarkdown(messages: Message[]): string {
  return messages
    .filter((m) => !m.pending && m.text.trim().length > 0)
    .map(blockFor)
    .join("\n\n");
}
