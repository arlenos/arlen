# Arlen terminal shell integration (TM-R3): emit the OSC 133/633/7 semantic-prompt
# marks the Arlen VT engine frames into command blocks (terminal.md §4.1).
#
# The marks carry the per-session nonce ($ARLEN_TERM_NONCE, minted and exported by
# the engine) on the 633;E command line, so terminal OUTPUT cannot forge a command
# boundary - only this trusted script knows the nonce, and the engine's scanner
# rejects a 633;E whose nonce does not match.
#
# Marks are written to a dedicated /dev/tty fd opened close-on-exec, so they reach
# the terminal even when a command redirects stdout (`build > log`) and are not
# inherited by child processes. With no nonce in the environment the engine is not
# driving this shell, so the integration stays completely silent.
#
# This file is sourced by the curated zsh (TM-R2 bakes it into the base image's
# zshrc); it installs precmd/preexec hooks and defines the mark emitters.

if [[ -n ${ARLEN_TERM_NONCE:-} ]]; then
  zmodload zsh/system 2>/dev/null
  if zmodload -e zsh/system 2>/dev/null \
     && sysopen -o cloexec -w -u _ARLEN_TERM_FD /dev/tty 2>/dev/null; then
    : # preferred: a close-on-exec write fd on the controlling tty
  elif exec {_ARLEN_TERM_FD}>/dev/tty 2>/dev/null; then
    : # fallback when zsh/system is unavailable (not cloexec; best effort)
  else
    unset _ARLEN_TERM_FD # no controlling tty -> integration disabled
  fi

  if [[ -n ${_ARLEN_TERM_FD:-} ]]; then
    typeset -g _ARLEN_TERM_RUNNING=0

    # Write one OSC sequence (number + payload) to the mark fd, BEL-terminated.
    # `-r` is load-bearing: without it `print` would re-interpret the \xHH escapes
    # in the payload and turn them back into the raw bytes the escaping removed
    # (a \x07 would become a real BEL and terminate the OSC early). The ESC] and
    # BEL framing come from the shell's own $'...' so they are real bytes; the
    # payload stays literal for the engine's scanner to decode.
    _arlen_term_osc() { print -rnu $_ARLEN_TERM_FD -- $'\e]'"$1"$'\a' }

    # preexec: a command is about to run. Report its exact command line - escaped
    # the way the engine decodes it (\xHH for backslash FIRST, then the field
    # separator and newlines, so a command containing ';' round-trips and cannot
    # smuggle the nonce field) - tagged with the nonce, then the exec-start mark.
    _arlen_term_preexec() {
      local s=$1
      s=${s//\\/\\x5c}        # backslash FIRST, so the escapes below are not re-escaped
      s=${s//;/\\x3b}         # the field separator (keeps the nonce field intact)
      s=${s//$'\n'/\\x0a}
      s=${s//$'\r'/\\x0d}
      s=${s//$'\a'/\\x07}     # BEL would terminate the OSC early -> escape it
      s=${s//$'\e'/\\x1b}     # ESC could start an ST/abort -> escape it
      _arlen_term_osc "633;E;${s};${ARLEN_TERM_NONCE}"
      _arlen_term_osc "133;C"
      _ARLEN_TERM_RUNNING=1
    }

    # precmd: a prompt is about to be drawn. Close the previous command with its
    # exit code (the ;D synthesis: only when a preexec actually ran, so an empty
    # line or an interrupted edit emits no spurious close), then open the new
    # prompt block and report the working directory.
    _arlen_term_precmd() {
      local exit=$?
      if (( _ARLEN_TERM_RUNNING )); then
        _arlen_term_osc "133;D;${exit}"
        _ARLEN_TERM_RUNNING=0
      fi
      _arlen_term_osc "633;A"
      _arlen_term_osc "7;file://${HOST}${PWD}"
    }

    autoload -Uz add-zsh-hook
    add-zsh-hook preexec _arlen_term_preexec
    add-zsh-hook precmd _arlen_term_precmd
  fi
fi
