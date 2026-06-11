# Arlen curated zsh (TM-R2)

The default shell experience: delightful by default, uncrippling by rule
(`terminal.md` §3). These files are the SOURCE the base image bakes; the runtime
artifacts land under `/usr/share/arlen/zsh/`.

## Files

- `zshrc` - the curated `.zshrc`. Sources the warm bundle, inits starship / fzf /
  zoxide, applies the interactive-only coreutils aliases, and sources the
  `arlen-shell-integration.zsh` block-mark seam (TM-R3). Every init is guarded by
  `command -v`, so a missing tool degrades to a plain working shell.
- `zsh_plugins.txt` - the antidote default plugin list (the zsh-function plugins
  only; binaries are packaged and wired in `zshrc`).

`arlen-shell-integration.zsh` (the TM-R3 mark emitter) lives in
`apps/terminal/integration/` and is installed alongside these into
`/usr/share/arlen/zsh/`.

## Image build (Phase 10)

The plugin set is resolved, ordered and wordcode-compiled ONCE at image build so
the first shell start is already warm (`zsh-bench`: the fast managers win by
paying the parse/compile cost at build time):

```sh
install -Dm644 zshrc                 "$ROOT/etc/skel/.zshrc"
install -Dm644 zsh_plugins.txt       "$ROOT/usr/share/arlen/zsh/zsh_plugins.txt"
install -Dm644 ../integration/arlen-shell-integration.zsh \
                                     "$ROOT/usr/share/arlen/zsh/arlen-shell-integration.zsh"

# Resolve + order + compile the static bundle (antidote, pure-zsh, static):
antidote bundle < zsh_plugins.txt >  "$ROOT/usr/share/arlen/zsh/bundle.zsh"
zsh -c "zcompile $ROOT/usr/share/arlen/zsh/bundle.zsh"
```

A user who adds plugins edits `zsh_plugins.txt` (or their own `~/.zshrc`);
antidote then loads the list live until the next image bake.

## Override

A power user's own `~/.zshrc` replaces this entirely - the curated set is the
default for the spectrum, not a cage.
