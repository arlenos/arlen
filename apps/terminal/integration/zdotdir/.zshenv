# Arlen curated ZDOTDIR (TM-R2), part of the shell-integration injection.
#
# The terminal engine starts zsh with ZDOTDIR pointed at this directory so the
# OSC block-mark integration (.zshrc) is sourced WITHOUT replacing the user's own
# zsh configuration. ARLEN_USER_ZDOTDIR holds the user's real config dir (the
# engine passes the parent ZDOTDIR there, or it falls back to $HOME).
#
# zsh reads $ZDOTDIR/.zshenv first. Source the user's .zshenv with their own
# ZDOTDIR in scope (so PATH and friends apply exactly as normal), then restore
# ZDOTDIR to this curated dir so zsh next reads THIS dir's .zshrc.
ARLEN_CURATED_ZDOTDIR=$ZDOTDIR
ZDOTDIR=${ARLEN_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zshenv ]] && source $ZDOTDIR/.zshenv
ZDOTDIR=$ARLEN_CURATED_ZDOTDIR
