---
name: meeting-prep
description: Before a calendar meeting, gather related notes and files and offer them.
kind: agent
reads: project
mode: suggest
trigger:
  type: event
  event: calendar.event.upcoming
tools:
  graph.query: []
budget:
  max_steps: 10
  max_tokens: 12000
  max_wall_ms: 15000
terminal:
  suggestion_ready: push
  nothing_relevant_found: silent
---

# meeting-prep

A read-only, Suggest-only behaviour. When a calendar event is upcoming
(`arlen-calendard` emits `calendar.event.upcoming` fifteen minutes before),
find related files, notes, and past meetings in the Knowledge Graph and
assemble a compact prep suggestion.

The meeting itself ARRIVES WITH THE RUN - uid, title, location, when it
starts - so there is no calendar tool to call and none is declared. This
manifest used to name `calendar.read`, which nothing implemented: a
declared capability that does not exist reads as a working feature right up
until somebody depends on it.

Security note (validated in the dry-run): the event's title/description is
**external content** - anyone can send a calendar invite, so it is a prompt-
injection vector. It enters tagged as `EXTERNAL-CONTENT` (S18-A), is
screened by the S17 classifier, and - because this behaviour is Suggest-
only - it can never act on injected instructions; any future variant that
could act would hit the hardcoded external-content confirmation rule.

Surfacing: `nothing_relevant_found` is `silent` (the P3 value floor - do not
announce having found nothing); a real result pushes, subject to timing and
an expiry (a meeting-prep suggestion is worthless once the meeting starts,
gap F10). Needs `project`-scoped read; if the global read level is lower
the behaviour is disabled with an explanation (gap G3).
