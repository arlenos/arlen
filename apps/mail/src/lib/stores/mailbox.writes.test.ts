import { beforeEach, describe, expect, it, vi } from "vitest";
import { get } from "svelte/store";
// Type-only, so it does not load the module before the mocks above are in place.
import type { Envelope } from "./mailbox";

// The four mailbox writes, from the side the drives cannot reach. `drive-mail-mailbox`
// presses these against a real maildir and then reads the disk, which is the
// right instrument for "did it land"; what it cannot show is the refusal path
// (the host has to fail for a reason the app cannot cause) or the sample mailbox,
// which no drive runs. Both are here.
let answers: Record<string, () => Promise<unknown>> = {};

vi.mock("$lib/tauri", () => ({ tauriAvailable: true }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args?: Record<string, unknown>) => {
    calls.push([cmd, args ?? {}]);
    const a = answers[cmd];
    return a ? a() : Promise.reject(new Error(`no answer for ${cmd}`));
  },
}));

const calls: [string, Record<string, unknown>][] = [];

const {
  envelopes,
  folders,
  mailboxState,
  markRead,
  moveMessage,
  deleteMessage,
  saveDraft,
} = await import("./mailbox");

/// A live mailbox with one unread message, an archive and a trash - the shape a
/// maildir has, where the folder ids are directory names and not rail names.
function aLiveMailbox(): void {
  calls.length = 0;
  answers = {};
  folders.set([
    { id: "inbox", kind: "inbox", unread: 1 },
    { id: ".Archive", kind: "archive", unread: 0 },
    { id: ".Trash", kind: "trash", unread: 0 },
  ]);
  envelopes.set([
    {
      id: "new/1.host",
      folderId: "inbox",
      from: "bank@example.com",
      subject: "your statement",
      snippet: "ready",
      dateMs: 1,
      unread: true,
    },
  ]);
  mailboxState.set("live");
}

describe("a write that lands", () => {
  beforeEach(aLiveMailbox);

  it("keeps the name the surface holds while the file moves under it", async () => {
    // The maildir puts flags in the filename, so reading renames the file. The
    // row must keep its id: the page's selection is keyed by it, and updating it
    // in place made the toolbar vanish the moment somebody clicked a message.
    answers["mail_mark_seen"] = () => Promise.resolve("cur/1.host:2,S");
    await markRead("new/1.host");
    const row = get(envelopes)[0];
    expect(row.id).toBe("new/1.host");
    expect(row.place).toBe("cur/1.host:2,S");
    expect(row.unread).toBe(false);
  });

  it("asks the mailbox about the folder it has, not the one the rail is called", async () => {
    // "archive" is what the surface says. `.Archive` is what the maildir has, and
    // the sample's ids happen to be the rail names - so a store that passed the
    // caller's word through would work on the fixture and never on a real one.
    answers["mail_move"] = () => Promise.resolve(".Archive/new/1.host");
    await moveMessage("new/1.host", "archive");
    expect(calls.find(([c]) => c === "mail_move")?.[1]).toEqual({
      id: "new/1.host",
      folderId: ".Archive",
    });
    expect(get(envelopes)[0].folderId).toBe(".Archive");
  });

  it("addresses the file where it is now, not where it first was", async () => {
    answers["mail_mark_seen"] = () => Promise.resolve("cur/1.host:2,S");
    await markRead("new/1.host");
    answers["mail_move"] = () => Promise.resolve(".Archive/cur/1.host:2,S");
    await moveMessage("new/1.host", "archive");
    expect(calls.filter(([c]) => c === "mail_move")[0][1]).toEqual({
      id: "cur/1.host:2,S",
      folderId: ".Archive",
    });
  });

  it("takes the row away when the message left the disk, and moves it when it did not", async () => {
    answers["mail_delete"] = () => Promise.resolve(".Trash/new/1.host");
    await deleteMessage("new/1.host");
    expect(get(envelopes)[0].folderId).toBe(".Trash");

    answers["mail_delete"] = () => Promise.resolve(null);
    await deleteMessage("new/1.host");
    expect(get(envelopes)).toHaveLength(0);
  });
});

describe("a write the mailbox refuses", () => {
  beforeEach(aLiveMailbox);

  it("leaves the dot where it was", async () => {
    // The optimistic write, and here it means a message somebody believes they
    // have read coming back unread at the next start.
    answers["mail_mark_seen"] = () => Promise.reject(new Error("not written"));
    await markRead("new/1.host");
    expect(get(envelopes)[0].unread).toBe(true);
    expect(get(envelopes)[0].place).toBeUndefined();
  });

  it("leaves the message in the folder it is in", async () => {
    answers["mail_move"] = () => Promise.reject(new Error("not written"));
    await moveMessage("new/1.host", "archive");
    expect(get(envelopes)[0].folderId).toBe("inbox");
  });

  it("leaves the row in the list", async () => {
    answers["mail_delete"] = () => Promise.reject(new Error("not written"));
    await deleteMessage("new/1.host");
    expect(get(envelopes)).toHaveLength(1);
  });

  it("answers a refused draft with nothing rather than an id", async () => {
    answers["mail_draft_save"] = () => Promise.reject(new Error("not written"));
    expect(await saveDraft("ada@example.org", "Later", "half a thought")).toBeNull();
  });

  it("does not move a message to a folder this mailbox does not have", async () => {
    // No `.Drafts` in this mailbox, so there is nothing to move to and nothing is
    // asked of the host - the refusal is decided before the wire, not after.
    await moveMessage("new/1.host", "drafts");
    expect(calls.some(([c]) => c === "mail_move")).toBe(false);
    expect(get(envelopes)[0].folderId).toBe("inbox");
  });
});

describe("the sample mailbox, which no drive runs", () => {
  beforeEach(() => {
    calls.length = 0;
    answers = {};
    folders.set([
      { id: "inbox", kind: "inbox", unread: 1 },
      { id: "trash", kind: "trash", unread: 0 },
    ]);
    envelopes.set([
      {
        id: "m1",
        folderId: "inbox",
        from: "rosa@example.org",
        subject: "the roof survey",
        snippet: "attached",
        dateMs: 1,
        unread: true,
      } satisfies Envelope,
    ]);
    mailboxState.set("sample");
  });

  it("moves a message to the trash and only then takes it away", async () => {
    // The rule lives in the store for both mailboxes. The page used to decide it
    // by comparing the open folder against the word "trash", which is true of
    // this mailbox and of no maildir.
    await deleteMessage("m1");
    expect(get(envelopes)[0].folderId).toBe("trash");
    await deleteMessage("m1");
    expect(get(envelopes)).toHaveLength(0);
  });

  it("asks the host for nothing", async () => {
    await markRead("m1");
    await deleteMessage("m1");
    expect(calls).toHaveLength(0);
    expect(get(envelopes)[0]?.unread ?? false).toBe(false);
  });
});
