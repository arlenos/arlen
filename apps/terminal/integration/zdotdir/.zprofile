# Arlen curated ZDOTDIR (TM-R2), login-shell leg of the injection.
#
# A login shell reads .zprofile between .zshenv and .zshrc. Source the user's
# .zprofile with their ZDOTDIR in scope, then restore ZDOTDIR to this curated dir
# so zsh still reads THIS dir's .zshrc next (where the integration is sourced).
ARLEN_CURATED_ZDOTDIR=$ZDOTDIR
ZDOTDIR=${ARLEN_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zprofile ]] && source $ZDOTDIR/.zprofile
ZDOTDIR=$ARLEN_CURATED_ZDOTDIR
