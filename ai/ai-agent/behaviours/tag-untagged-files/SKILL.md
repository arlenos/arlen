---
name: tag-untagged-files
description: Tag files that belong to a project but carry no membership edge yet.
kind: workflow
reads: project
mode: supervised
handler: tag_untagged_files
trigger:
  type: manual
tools:
  graph.query: []
  graph.write: [Project, FILE_PART_OF]
terminal:
  no_untagged_file: silent
---

# tag-untagged-files

A deterministic pull-mode workflow (no LLM). Invoked on demand, it scans the
graph for a File that lies under a known `Project` root but carries no live
`FILE_PART_OF` edge and proposes creating that edge. It is the manual
counterpart of `auto-tag-by-project`, which reacts to each `file.opened`;
this one is run by the user to curate files that accumulated untagged.

It proposes the single most-specific match for the first untagged file it
finds (the same longest-prefix, no-guess-on-ties rule as auto-tag) and reaches
the `no_untagged_file` terminal otherwise; re-invoking proposes the next.

Because it is manually invoked rather than externally triggered, and the
proposal is provable (the File and Project already exist and no edge is
present), the gate lifts it to a previewed execution instead of holding it for
an external-trigger confirmation.
