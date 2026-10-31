//! Shell completion scripts. Each calls `rexinator list --names` for project names.

use crate::cli::Shell;

pub fn completions(shell: Shell) -> &'static str {
    match shell {
        Shell::Zsh => {
            r#"#compdef rexinator
_rexinator() {
  local -a cmds
  cmds=(
    'start:Start a project, or switch to it'
    'stop:Stop a project'
    'debug:Print the request sent to Rex'
    'list:List projects'
    'edit:Edit a project file'
    'new:Create a project file'
    'copy:Copy a project file'
    'delete:Delete a project file'
    'completions:Print a shell completion script'
  )
  if (( CURRENT == 2 )); then
    _describe 'command' cmds
    return
  fi
  case $words[2] in
    start|s|stop|debug|edit|new|open|copy|cp|delete|rm)
      [[ $PREFIX == -* ]] || compadd -- ${(f)"$(rexinator list --names 2>/dev/null)"} ;;
    completions) compadd bash zsh fish ;;
  esac
}
compdef _rexinator rexinator
"#
        }
        Shell::Bash => {
            r#"_rexinator() {
  local cur=${COMP_WORDS[COMP_CWORD]}
  if [ "$COMP_CWORD" -eq 1 ]; then
    COMPREPLY=($(compgen -W "start stop debug list edit new copy delete completions" -- "$cur"))
    return
  fi
  case ${COMP_WORDS[1]} in
    start|s|stop|debug|edit|new|open|copy|cp|delete|rm)
      [[ $cur == -* ]] || COMPREPLY=($(compgen -W "$(rexinator list --names 2>/dev/null)" -- "$cur")) ;;
    completions) COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur")) ;;
  esac
}
complete -F _rexinator rexinator
"#
        }
        Shell::Fish => {
            r#"set -l cmds start stop debug list edit new copy delete completions
complete -c rexinator -f
complete -c rexinator -n "not __fish_seen_subcommand_from $cmds s ls open cp rm" -a "$cmds"
complete -c rexinator -n "__fish_seen_subcommand_from start s stop debug edit new open copy cp delete rm" -a "(rexinator list --names 2>/dev/null)"
complete -c rexinator -n "__fish_seen_subcommand_from completions" -a "bash zsh fish"
"#
        }
    }
}
