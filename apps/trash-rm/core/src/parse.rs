//! Parse a `rm`-compatible argument vector.
//!
//! Accepts the POSIX + common GNU `rm` flags so the trash-first delete is a drop-in:
//! `-r`/`-R`/`--recursive`, `-f`/`--force`, `-i` (interactive), `-d`/`--dir` (remove
//! empty directories), `-v`/`--verbose`, `--one-file-system`, `--preserve-root`
//! (default) / `--no-preserve-root`, and `--` (end of options). A lone `-` and any
//! non-`-` argument is an operand. Short flags combine (`-rf`). An unknown flag is a
//! hard error (like `rm`), so the delete never silently ignores an option it does not
//! model. Pure: no filesystem access.

/// A parsed `rm` invocation: the operands plus the behaviour flags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RmInvocation {
    /// The paths to delete, in order.
    pub paths: Vec<String>,
    /// `-r`/`-R`: recurse into directories.
    pub recursive: bool,
    /// `-f`: ignore nonexistent operands, never prompt, and make an empty operand
    /// list a success no-op (POSIX).
    pub force: bool,
    /// `-i`: prompt before each removal.
    pub interactive: bool,
    /// `-d`/`--dir`: remove empty directories as well as files.
    pub dir: bool,
    /// `-v`: explain what is being done.
    pub verbose: bool,
    /// `--one-file-system`: skip directories on a different filesystem.
    pub one_file_system: bool,
    /// `--no-preserve-root`: allow operating on `/` (default refuses it).
    pub no_preserve_root: bool,
    /// `--purge`: an explicit hard-delete escape hatch (Arlen extension) - unlink,
    /// never trash. The routing layer also hard-deletes non-interactively.
    pub purge: bool,
}

/// Why an argument vector could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RmError {
    /// An option the delete does not model (fail loud, never silently ignore).
    UnknownFlag(String),
    /// No operands given and not `-f` (POSIX: "missing operand").
    MissingOperand,
    /// An operand is `/` and `--no-preserve-root` was not given.
    RefusedRoot,
}

/// Parse `args` (the arguments AFTER the program name) into an [`RmInvocation`],
/// or an [`RmError`]. Faithful to `rm`'s option grammar so scripts behave.
pub fn parse_rm_args(args: &[String]) -> Result<RmInvocation, RmError> {
    let mut inv = RmInvocation::default();
    let mut options_ended = false;

    for arg in args {
        if options_ended {
            inv.paths.push(arg.clone());
            continue;
        }
        if arg == "--" {
            options_ended = true;
            continue;
        }
        // A lone "-" and anything not starting with "-" is an operand.
        if arg == "-" || !arg.starts_with('-') {
            inv.paths.push(arg.clone());
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "recursive" => inv.recursive = true,
                "force" => inv.force = true,
                "interactive" => inv.interactive = true,
                "dir" => inv.dir = true,
                "verbose" => inv.verbose = true,
                "one-file-system" => inv.one_file_system = true,
                "preserve-root" => inv.no_preserve_root = false,
                "no-preserve-root" => inv.no_preserve_root = true,
                "purge" => inv.purge = true,
                _ => return Err(RmError::UnknownFlag(arg.clone())),
            }
            continue;
        }
        // Short flags, possibly combined (e.g. `-rf`).
        for c in arg[1..].chars() {
            match c {
                'r' | 'R' => inv.recursive = true,
                'f' => inv.force = true,
                'i' => inv.interactive = true,
                'd' => inv.dir = true,
                'v' => inv.verbose = true,
                _ => return Err(RmError::UnknownFlag(format!("-{c}"))),
            }
        }
    }

    if inv.paths.is_empty() && !inv.force {
        return Err(RmError::MissingOperand);
    }
    if !inv.no_preserve_root && inv.paths.iter().any(|p| p == "/") {
        return Err(RmError::RefusedRoot);
    }
    Ok(inv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_paths_and_combined_short_flags() {
        let inv = parse_rm_args(&args(&["-rf", "a", "b/c"])).unwrap();
        assert!(inv.recursive && inv.force);
        assert_eq!(inv.paths, vec!["a", "b/c"]);
    }

    #[test]
    fn long_flags_and_double_dash_operands() {
        let inv = parse_rm_args(&args(&["--recursive", "--", "-weird-name", "x"])).unwrap();
        assert!(inv.recursive);
        // After `--`, a leading-dash name is an operand, not a flag.
        assert_eq!(inv.paths, vec!["-weird-name", "x"]);
    }

    #[test]
    fn a_lone_dash_is_an_operand() {
        let inv = parse_rm_args(&args(&["-"])).unwrap();
        assert_eq!(inv.paths, vec!["-"]);
    }

    #[test]
    fn unknown_flag_is_a_hard_error() {
        assert_eq!(
            parse_rm_args(&args(&["-rz", "a"])),
            Err(RmError::UnknownFlag("-z".into()))
        );
        assert_eq!(
            parse_rm_args(&args(&["--frobnicate"])),
            Err(RmError::UnknownFlag("--frobnicate".into()))
        );
    }

    #[test]
    fn missing_operand_errors_unless_force() {
        assert_eq!(parse_rm_args(&args(&[])), Err(RmError::MissingOperand));
        // `rm -f` with no operands is a success no-op.
        assert_eq!(parse_rm_args(&args(&["-f"])).unwrap().paths, Vec::<String>::new());
    }

    #[test]
    fn root_is_refused_unless_no_preserve_root() {
        assert_eq!(parse_rm_args(&args(&["-rf", "/"])), Err(RmError::RefusedRoot));
        let inv = parse_rm_args(&args(&["-rf", "--no-preserve-root", "/"])).unwrap();
        assert_eq!(inv.paths, vec!["/"]);
    }

    #[test]
    fn purge_is_parsed() {
        assert!(parse_rm_args(&args(&["--purge", "a"])).unwrap().purge);
    }
}
