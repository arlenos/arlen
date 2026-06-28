---
name: tidy-downloads
description: Sort files in ~/Downloads into project folders by inferred topic.
kind: agent
reads: full
mode: supervised
trigger:
  type: schedule
  every_secs: 604800
tools:
  graph.query: []
  fs.list: [~/Downloads]
  fs.move: [~/Downloads, ~/Documents/Projects]
budget:
  max_steps: 40
  max_tokens: 30000
  max_wall_ms: 120000
terminal:
  downloads_empty_or_unsortable: store
  no_confident_moves: silent
---

# tidy-downloads

A bounded agentic loop. For each file in `~/Downloads`, infer the best
destination project folder from the filename plus Knowledge-Graph context;
move only high-confidence files and leave the rest.

Safety notes (from the dry-run):

- A `move` onto an occupied destination never overwrites: the executor
  renames the moved file to the first free sibling name (`report (1).pdf`),
  so the move always carries an exact restore-path inverse and stays
  reversible (gap F4, fixed in the move planner). A collision is handled
  silently and safely, not skipped or escalated. A Snapper snapshot before
  the batch is still the cleaner way to undo the whole move set at once
  (gap B1).
- The cadence above is weekly; actual firing is gated to an idle window by
  the idle scheduler (B3). The B0 schema only expresses the interval; the
  "and idle" qualifier is the scheduler's concern, not the manifest's.
- Default mode is `supervised`, not `autonomous`: silently rearranging a
  user's files, even reversibly, must be a deliberate per-app opt-in, and
  even then should emit a post-hoc "tidied N files → Undo" summary (gap F7).
