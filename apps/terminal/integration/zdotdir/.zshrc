# Arlen curated ZDOTDIR (TM-R2), the interactive leg of the injection.
#
# Restore the user's real config dir, run their .zshrc exactly as in any other
# terminal (their prompt, plugins and hooks load with the correct ZDOTDIR), then
# source the Arlen shell integration LAST. The integration only emits the OSC
# block marks when ARLEN_TERM_NONCE is set (the engine sets it), so it stays
# silent in a shell the Arlen terminal is not driving, and add-zsh-hook composes
# with the user's own precmd/preexec hooks rather than replacing them.
#
# ZDOTDIR is deliberately left at the user's value after this: the interactive
# session should report the user's config dir, and any login .zlogin zsh reads
# after .zshrc then comes from the user's dir automatically.
ARLEN_CURATED_ZDOTDIR=$ZDOTDIR
ZDOTDIR=${ARLEN_USER_ZDOTDIR:-$HOME}
[[ -f $ZDOTDIR/.zshrc ]] && source $ZDOTDIR/.zshrc
source $ARLEN_CURATED_ZDOTDIR/arlen-shell-integration.zsh
