# Skills

Agent instructions that the Claude Code harness loads from `~/.claude/skills/`. They live here instead, and
that directory holds symlinks into this one.

**Why they moved (7 September).** `~/.claude/skills/` is not a git repository and is in neither of this
project's trees: no history, no review, no attribution, no backup. And because every agent on the machine
shares it, one lane could edit another lane's instructions by accident — the same coupling the repository split
exists to prevent. Authoring them in the repo whose tooling they describe fixes both, and settles who may edit
which.

**The symlink is load-bearing.** The harness reads `~/.claude/skills/<name>/SKILL.md` and nothing else, so a
fresh machine needs the links recreated; `dev/setup-skills.sh` does that and is idempotent.

**What belongs here:** a skill about this repository's tooling. A skill about the compositor's belongs in the
compositor repo beside the scripts it describes. Personal skills that have nothing to do with this project stay
in `~/.claude/skills/` as ordinary directories; the setup script leaves anything it did not create alone.
