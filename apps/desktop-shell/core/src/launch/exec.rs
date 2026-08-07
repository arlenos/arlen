//! Turning a desktop entry's `Exec` value into an argument vector.
//!
//! The property this exists to hold: **a file name becomes exactly one argument,
//! whatever is in it.** A document called `; rm -rf ~` or `--privileged` is a
//! document, and the only way it stays one is if expansion happens after the
//! command line has already been split, never before. So the value is tokenised
//! first and the field codes are filled into the tokens - there is no point at
//! which a caller-supplied name is re-split, re-quoted or handed to a shell.
//!
//! This replaces stripping. The shell's app index removes `%f` and friends
//! outright, which is right for "start this application" and is exactly why
//! nothing here can open a document with one: by launch time the placeholder is
//! gone. Expanding them is the piece the launch service needs.
//!
//! Spec: <https://specifications.freedesktop.org/desktop-entry-spec/latest/ar01s07.html>
//!
//! Takes the value **after** desktop-file unescaping (`\\s`, `\\n`, `\\t`, `\\r`,
//! `\\\\`), which belongs to whatever parsed the entry.

use std::fmt;

/// A document to hand the application, in the two forms the field codes want.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The URI, for `%u` / `%U`.
    pub uri: String,
    /// The local path, for `%f` / `%F`. `None` for a target that is not a local
    /// file, in which case an application that only takes `%f` cannot open it.
    pub path: Option<String>,
}

/// Everything the non-document field codes need.
///
/// Each is optional because a desktop entry need not have the key, and a missing
/// key means the code expands to nothing rather than to an empty argument - an
/// empty argument is a real argument, and passing one the entry never asked for
/// is how `--icon ""` reaches a program.
#[derive(Debug, Default, Clone)]
pub struct ExecContext<'a> {
    /// Documents to open. Empty means "just start it".
    pub targets: &'a [Target],
    /// The entry's `Icon`, for `%i`.
    pub icon: Option<&'a str>,
    /// The entry's translated `Name`, for `%c`.
    pub name: Option<&'a str>,
    /// The path of the desktop file itself, for `%k`.
    pub desktop_file: Option<&'a str>,
}

/// Why an `Exec` value could not be turned into an argument vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// Nothing to run.
    Empty,
    /// A quote is opened and never closed, so where the arguments end is a
    /// guess. Guessing here would mean running a command line the entry did not
    /// write.
    UnterminatedQuote,
    /// A trailing backslash with nothing to escape, same reasoning.
    DanglingEscape,
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "the Exec value is empty"),
            Self::UnterminatedQuote => write!(f, "the Exec value has an unterminated quote"),
            Self::DanglingEscape => write!(f, "the Exec value ends in a backslash"),
        }
    }
}

impl std::error::Error for ExecError {}

/// One piece of a token: literal text, or a field code to fill in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Code(char),
}

/// Split an `Exec` value into tokens, keeping unquoted field codes as codes.
///
/// A `%` inside quotes stays literal: the spec says field codes must not appear
/// inside a quoted argument, and reading one as literal text is the reading that
/// cannot turn quoted data into a substitution.
fn tokenize(exec: &str) -> Result<Vec<Vec<Piece>>, ExecError> {
    let mut tokens: Vec<Vec<Piece>> = Vec::new();
    let mut token: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = exec.chars().peekable();

    macro_rules! flush_text {
        () => {
            if !text.is_empty() {
                token.push(Piece::Text(std::mem::take(&mut text)));
            }
        };
    }

    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // Escapes the next character, in or out of quotes. Whatever it
                // is, it is data: this is how a name containing a quote or a
                // space survives as part of one argument.
                let Some(next) = chars.next() else {
                    return Err(ExecError::DanglingEscape);
                };
                text.push(next);
                started = true;
            }
            '"' | '\'' => {
                match quote {
                    Some(q) if q == c => quote = None,
                    Some(_) => text.push(c),
                    None => quote = Some(c),
                }
                started = true;
            }
            '%' if quote.is_none() => {
                match chars.next() {
                    // `%%` is a literal percent, not an empty expansion.
                    Some('%') => text.push('%'),
                    Some(code) => {
                        flush_text!();
                        token.push(Piece::Code(code));
                    }
                    // A trailing bare `%` is not a code; keep it as text.
                    None => text.push('%'),
                }
                started = true;
            }
            c if c.is_whitespace() && quote.is_none() => {
                if started {
                    flush_text!();
                    tokens.push(std::mem::take(&mut token));
                    started = false;
                }
            }
            c => {
                text.push(c);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(ExecError::UnterminatedQuote);
    }
    if started {
        flush_text!();
        tokens.push(token);
    }
    Ok(tokens)
}

/// Expand an `Exec` value into the argument vector to run.
///
/// Field codes are filled in after splitting, so an expanded value is one
/// argument no matter what it contains. `%F` and `%U` expand to one argument per
/// target; every other code expands to at most one.
///
/// **A token holding a code that cannot be filled is dropped whole**, not filled
/// with nothing. `--file=%f` with no document must not become `--file=`: an
/// empty argument is a real argument, and one the entry never asked to pass.
/// The same rule covers `%i` without an icon, the deprecated codes (`%d`, `%D`,
/// `%n`, `%N`, `%v`, `%m`) and unknown ones, which the spec says to remove - and
/// removing is the safe direction anyway, since a passed-through `%z` would
/// reach the program as literal text it never expected.
///
/// The list codes are only honoured as a token of their own. `--files=%F` cannot
/// mean a list, and joining one into the surrounding text would produce a single
/// argument holding several file names - the exact shape this module exists to
/// prevent - so that token drops like any other unfillable one.
pub fn expand_exec(exec: &str, ctx: &ExecContext<'_>) -> Result<Vec<String>, ExecError> {
    let tokens = tokenize(exec)?;
    if tokens.is_empty() {
        return Err(ExecError::Empty);
    }

    // What a single code expands to, or `None` if it has nothing to say.
    let fill = |c: char| -> Option<String> {
        match c {
            'f' => ctx.targets.first().and_then(|t| t.path.clone()),
            'u' => ctx.targets.first().map(|t| t.uri.clone()),
            'c' => ctx.name.map(str::to_string),
            'k' => ctx.desktop_file.map(str::to_string),
            // The list codes and `%i` are not single values; they are handled as
            // whole tokens above. Deprecated and unknown codes have no value.
            _ => None,
        }
    };

    let mut args: Vec<String> = Vec::new();
    for token in &tokens {
        // The codes that expand to something other than one argument are only
        // meaningful alone.
        if let [Piece::Code(c)] = token.as_slice() {
            match c {
                'F' => {
                    args.extend(ctx.targets.iter().filter_map(|t| t.path.clone()));
                    continue;
                }
                'U' => {
                    args.extend(ctx.targets.iter().map(|t| t.uri.clone()));
                    continue;
                }
                'i' => {
                    if let Some(icon) = ctx.icon {
                        args.push("--icon".to_string());
                        args.push(icon.to_string());
                    }
                    continue;
                }
                _ => {}
            }
        }

        let mut out = String::new();
        let mut fillable = true;
        for piece in token {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Code(c) => match fill(*c) {
                    Some(v) => out.push_str(&v),
                    None => {
                        fillable = false;
                        break;
                    }
                },
            }
        }
        if fillable {
            args.push(out);
        }
    }

    if args.is_empty() {
        return Err(ExecError::Empty);
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> Target {
        Target {
            uri: format!("file://{path}"),
            path: Some(path.to_string()),
        }
    }

    fn ctx<'a>(targets: &'a [Target]) -> ExecContext<'a> {
        ExecContext {
            targets,
            ..Default::default()
        }
    }

    #[test]
    fn a_plain_command_splits_on_whitespace() {
        assert_eq!(
            expand_exec("prog --flag value", &ctx(&[])).unwrap(),
            ["prog", "--flag", "value"]
        );
    }

    #[test]
    fn quoted_arguments_keep_their_spaces() {
        assert_eq!(
            expand_exec("prog \"a b\" 'c d'", &ctx(&[])).unwrap(),
            ["prog", "a b", "c d"]
        );
    }

    /// The property the module exists for.
    #[test]
    fn a_file_name_full_of_shell_syntax_is_still_one_argument() {
        let t = [file("/tmp/; rm -rf ~ && echo $HOME | tee x")];
        assert_eq!(
            expand_exec("prog %f", &ctx(&t)).unwrap(),
            ["prog", "/tmp/; rm -rf ~ && echo $HOME | tee x"]
        );
    }

    /// A name that looks like a flag is a name.
    #[test]
    fn a_file_name_that_looks_like_a_flag_is_not_split_off() {
        let t = [file("/tmp/--privileged")];
        let args = expand_exec("prog %f", &ctx(&t)).unwrap();
        assert_eq!(args, ["prog", "/tmp/--privileged"]);
        assert_eq!(args.len(), 2);
    }

    #[test]
    fn a_list_code_expands_to_one_argument_per_target() {
        let t = [file("/a"), file("/b")];
        assert_eq!(
            expand_exec("prog %F", &ctx(&t)).unwrap(),
            ["prog", "/a", "/b"]
        );
        assert_eq!(
            expand_exec("prog %U", &ctx(&t)).unwrap(),
            ["prog", "file:///a", "file:///b"]
        );
    }

    /// Several file names joined into one argument is the shape this prevents,
    /// so a list code welded to other text expands to nothing at all.
    #[test]
    fn a_list_code_inside_a_larger_token_is_dropped() {
        let t = [file("/a"), file("/b")];
        assert_eq!(expand_exec("prog --files=%F", &ctx(&t)).unwrap(), ["prog"]);
    }

    /// `--file=` is not "no file", it is an empty file name, and the entry never
    /// asked to pass one.
    #[test]
    fn a_code_that_cannot_be_filled_takes_its_whole_token_with_it() {
        assert_eq!(expand_exec("prog --file=%f", &ctx(&[])).unwrap(), ["prog"]);
        let c = ExecContext {
            name: Some("N"),
            ..Default::default()
        };
        assert_eq!(
            expand_exec("prog --n=%c --k=%k", &c).unwrap(),
            ["prog", "--n=N"]
        );
    }

    #[test]
    fn a_single_code_may_sit_inside_a_larger_argument() {
        let t = [file("/a b")];
        assert_eq!(
            expand_exec("prog --file=%f", &ctx(&t)).unwrap(),
            ["prog", "--file=/a b"]
        );
    }

    #[test]
    fn no_targets_means_the_placeholder_disappears() {
        assert_eq!(expand_exec("prog %f", &ctx(&[])).unwrap(), ["prog"]);
        assert_eq!(expand_exec("prog %U", &ctx(&[])).unwrap(), ["prog"]);
    }

    /// An application that only takes local paths cannot open a remote thing,
    /// and inventing a path for it would be worse than not opening it.
    #[test]
    fn a_non_local_target_does_not_satisfy_the_file_code() {
        let t = [Target {
            uri: "https://example.org/x".into(),
            path: None,
        }];
        assert_eq!(expand_exec("prog %f", &ctx(&t)).unwrap(), ["prog"]);
        assert_eq!(
            expand_exec("prog %u", &ctx(&t)).unwrap(),
            ["prog", "https://example.org/x"]
        );
    }

    #[test]
    fn only_the_first_target_fills_a_singular_code() {
        let t = [file("/a"), file("/b")];
        assert_eq!(expand_exec("prog %f", &ctx(&t)).unwrap(), ["prog", "/a"]);
    }

    #[test]
    fn the_icon_code_becomes_a_flag_and_its_value() {
        let c = ExecContext {
            icon: Some("some-icon"),
            ..Default::default()
        };
        assert_eq!(
            expand_exec("prog %i", &c).unwrap(),
            ["prog", "--icon", "some-icon"]
        );
    }

    #[test]
    fn the_icon_code_without_an_icon_adds_nothing() {
        assert_eq!(expand_exec("prog %i", &ctx(&[])).unwrap(), ["prog"]);
    }

    #[test]
    fn the_name_and_desktop_file_codes_expand() {
        let c = ExecContext {
            name: Some("A Name"),
            desktop_file: Some("/usr/share/applications/x.desktop"),
            ..Default::default()
        };
        assert_eq!(
            expand_exec("prog %c %k", &c).unwrap(),
            ["prog", "A Name", "/usr/share/applications/x.desktop"]
        );
    }

    #[test]
    fn deprecated_and_unknown_codes_are_removed() {
        assert_eq!(
            expand_exec("prog %d %D %n %N %v %m %z", &ctx(&[])).unwrap(),
            ["prog"]
        );
    }

    #[test]
    fn a_doubled_percent_is_a_literal_percent() {
        assert_eq!(
            expand_exec("prog 100%% done", &ctx(&[])).unwrap(),
            ["prog", "100%", "done"]
        );
    }

    /// A quoted `%f` is data, not a placeholder.
    #[test]
    fn a_code_inside_quotes_stays_literal() {
        let t = [file("/a")];
        assert_eq!(
            expand_exec("prog \"%f\"", &ctx(&t)).unwrap(),
            ["prog", "%f"]
        );
    }

    #[test]
    fn a_backslash_escapes_the_next_character() {
        assert_eq!(
            expand_exec(r#"prog a\ b "say \"hi\"""#, &ctx(&[])).unwrap(),
            ["prog", "a b", r#"say "hi""#]
        );
    }

    /// Where the arguments end must not be a guess.
    #[test]
    fn an_unterminated_quote_is_refused() {
        assert_eq!(
            expand_exec("prog \"unclosed", &ctx(&[])),
            Err(ExecError::UnterminatedQuote)
        );
    }

    #[test]
    fn a_trailing_backslash_is_refused() {
        assert_eq!(
            expand_exec(r"prog trailing\", &ctx(&[])),
            Err(ExecError::DanglingEscape)
        );
    }

    #[test]
    fn an_empty_value_is_refused() {
        assert_eq!(expand_exec("", &ctx(&[])), Err(ExecError::Empty));
        assert_eq!(expand_exec("   ", &ctx(&[])), Err(ExecError::Empty));
    }

    /// An entry whose whole command line is a placeholder leaves nothing to run,
    /// which is a refusal rather than an empty argv.
    #[test]
    fn a_value_that_expands_to_nothing_is_refused() {
        assert_eq!(expand_exec("%f", &ctx(&[])), Err(ExecError::Empty));
    }

    #[test]
    fn an_empty_quoted_argument_survives() {
        assert_eq!(
            expand_exec("prog \"\" tail", &ctx(&[])).unwrap(),
            ["prog", "", "tail"]
        );
    }

    #[test]
    fn leading_and_repeated_whitespace_makes_no_empty_arguments() {
        assert_eq!(
            expand_exec("  prog   a  ", &ctx(&[])).unwrap(),
            ["prog", "a"]
        );
    }
}
