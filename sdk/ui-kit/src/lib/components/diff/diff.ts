/// The change-diff model the `DiffView` renders: a set of changed files, each a
/// list of hunks of classified lines. A raw unified-diff string is also accepted
/// and parsed client-side, since that is what most edit tooling already
/// produces. Shared so any surface that shows a proposed or applied change (the
/// harness gate/receipt, a future reviewer) renders diffs the same way.

/// How a file is changed across the set.
export type FileStatus = "modified" | "added" | "deleted" | "renamed";

/// One classified diff line (the leading +/-/space is stripped into `kind`).
export interface DiffLine {
  kind: "add" | "del" | "context";
  text: string;
}

/// A contiguous run of changed lines under its `@@ … @@` header.
export interface DiffHunk {
  header: string;
  lines: DiffLine[];
}

/// One file's change: its path, status, hunks, and the +/- line counts.
export interface DiffFile {
  path: string;
  /// For a rename, the previous path.
  oldPath?: string;
  status: FileStatus;
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
}

/// The leading path of a `diff --git a/… b/…` or `+++ b/…` line, de-prefixed.
function stripPrefix(p: string): string {
  return p.replace(/^[ab]\//, "");
}

/// Parse a raw unified diff (git or plain `---`/`+++`/`@@`) into files. Tolerant:
/// an unrecognised line inside a hunk is treated as context, so a malformed diff
/// still renders rather than throwing.
export function parseUnifiedDiff(text: string): DiffFile[] {
  const files: DiffFile[] = [];
  let file: DiffFile | null = null;
  let hunk: DiffHunk | null = null;

  const pushFile = () => {
    if (file) files.push(file);
  };

  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git")) {
      pushFile();
      const m = line.match(/ a\/(.+) b\/(.+)$/);
      const path = m ? stripPrefix(`b/${m[2]}`) : "";
      file = { path, status: "modified", hunks: [], additions: 0, deletions: 0 };
      hunk = null;
      continue;
    }
    if (line.startsWith("new file")) {
      if (file) file.status = "added";
      continue;
    }
    if (line.startsWith("deleted file")) {
      if (file) file.status = "deleted";
      continue;
    }
    if (line.startsWith("rename from ")) {
      if (file) {
        file.status = "renamed";
        file.oldPath = line.slice("rename from ".length).trim();
      }
      continue;
    }
    if (line.startsWith("--- ")) {
      // A plain (non-git) diff opens a new file here.
      if (!file) {
        file = { path: "", status: "modified", hunks: [], additions: 0, deletions: 0 };
      }
      continue;
    }
    if (line.startsWith("+++ ")) {
      const p = stripPrefix(line.slice(4).trim());
      if (file && (!file.path || file.path === "")) file.path = p;
      continue;
    }
    if (line.startsWith("@@")) {
      if (!file) {
        file = { path: "", status: "modified", hunks: [], additions: 0, deletions: 0 };
      }
      hunk = { header: line, lines: [] };
      file.hunks.push(hunk);
      continue;
    }
    if (!file || !hunk) continue;
    if (line.startsWith("+")) {
      hunk.lines.push({ kind: "add", text: line.slice(1) });
      file.additions++;
    } else if (line.startsWith("-")) {
      hunk.lines.push({ kind: "del", text: line.slice(1) });
      file.deletions++;
    } else if (line.startsWith("\\")) {
      // "\ No newline at end of file" - metadata, skip.
      continue;
    } else {
      hunk.lines.push({ kind: "context", text: line.startsWith(" ") ? line.slice(1) : line });
    }
  }
  pushFile();
  return files.filter((f) => f.hunks.length > 0 || f.status !== "modified");
}

/// Total added/removed lines across a change set, for the summary line.
export function diffTotals(files: DiffFile[]): { additions: number; deletions: number } {
  return files.reduce(
    (acc, f) => ({ additions: acc.additions + f.additions, deletions: acc.deletions + f.deletions }),
    { additions: 0, deletions: 0 },
  );
}
