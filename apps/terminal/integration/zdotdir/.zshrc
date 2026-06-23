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

# A real terminal shows `ls` in colour (folders vs files). The engine advertises
# a colour TERM and carries SGR through to the grid, but `ls` only emits colour
# when asked, so ensure a palette + the colour alias as a terminal default. The
# user's own LS_COLORS / `ls` alias was sourced just above and wins; this only
# fills the gap when their config sets neither (a bare distro, a minimal dotfile).
[[ -z $LS_COLORS ]] && (( $+commands[dircolors] )) && eval "$(dircolors -b)"
(( $+aliases[ls] )) || alias ls='ls --color=auto'
# The integration script is the sibling of this curated dir (`:h` is the parent);
# guard so a missing script degrades to a plain shell instead of erroring at the
# user.
[[ -f ${ARLEN_CURATED_ZDOTDIR:h}/arlen-shell-integration.zsh ]] \
  && source ${ARLEN_CURATED_ZDOTDIR:h}/arlen-shell-integration.zsh
