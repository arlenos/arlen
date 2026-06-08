import { describe, it, expect } from "vitest";
import { conversationToMarkdown } from "./export";
import type { Message } from "$lib/stores/conversation";

function msg(partial: Partial<Message> & Pick<Message, "role" | "text">): Message {
  return { id: 0, ...partial };
}

describe("conversationToMarkdown", () => {
  it("renders a user/assistant exchange with role labels", () => {
    const md = conversationToMarkdown([
      msg({ id: 1, role: "user", text: "What is in my downloads?" }),
      msg({ id: 2, role: "assistant", text: "Three files." }),
    ]);
    expect(md).toBe(
      "**You:**\n\nWhat is in my downloads?\n\n**Assistant:**\n\nThree files.",
    );
  });

  it("skips pending and empty turns", () => {
    const md = conversationToMarkdown([
      msg({ id: 1, role: "user", text: "Hi" }),
      msg({ id: 2, role: "assistant", text: "", pending: true }),
      msg({ id: 3, role: "assistant", text: "   " }),
    ]);
    expect(md).toBe("**You:**\n\nHi");
  });

  it("labels an error turn as Error, not an answer", () => {
    const md = conversationToMarkdown([msg({ id: 1, role: "error", text: "daemon down" })]);
    expect(md).toBe("**Error:**\n\ndaemon down");
  });

  it("notes attachments on a user turn", () => {
    const md = conversationToMarkdown([
      msg({ id: 1, role: "user", text: "Summarise these", mentions: ["a.md", "b.md"] }),
    ]);
    expect(md).toBe("**You:**\n\nSummarise these\n\n_Attached: a.md, b.md_");
  });

  it("keeps an attachment-only user turn (no typed text)", () => {
    const md = conversationToMarkdown([
      msg({ id: 1, role: "user", text: "", mentions: ["report.md"] }),
      msg({ id: 2, role: "assistant", text: "Summary." }),
    ]);
    expect(md).toBe("**You:**\n\n_Attached: report.md_\n\n**Assistant:**\n\nSummary.");
  });

  it("is empty for an empty or all-pending conversation", () => {
    expect(conversationToMarkdown([])).toBe("");
    expect(conversationToMarkdown([msg({ id: 1, role: "assistant", text: "", pending: true })])).toBe("");
  });
});
