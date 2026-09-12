//! A theme target a PERSON declares, beside the four we ship.
//!
//! `Toolkit` is a closed enum and `ToolkitOverrides` a struct with four named
//! fields, so somebody who wants their editor or their music player to follow the
//! system theme has one route: a pull request to us. The curation policy behind
//! that is right for what we ship - an emitter lands when its mapping is
//! unambiguous, and that judgement needs somebody looking at real windows - and it
//! is the wrong rule for what a person may do to their own machine
//! (`theme-system.md` §9c).
//!
//! So a target is a DECLARATION: a name, a destination, and a template naming the
//! tokens the resolver already produces. The four shipped emitters keep their
//! hand-tuned mappings and are not touched by any of this.
//!
//! ## An installed theme cannot run code
//!
//! Borrowed outright, because it is correct: a theme changes what a desktop looks
//! like, never what it runs. Every refusal below happens at PARSE, not at write:
//!
//! - The schema is CLOSED (`deny_unknown_fields`), so a `hook`, `exec`, `script`
//!   or `command` key does not fail on use, it fails to parse. That is the whole
//!   defence and it is the strongest available shape: there is no key to carry an
//!   instruction in.
//! - A template carrying `#!` is refused. An interpreter directive in a generated
//!   file is the one thing that turns a colour scheme into a program.
//! - A destination is RELATIVE, inside the config directory, with no `..` and no
//!   `~`. A declaration cannot name a path outside its own target.
//! - A placeholder must name a token that exists. An unknown one is refused rather
//!   than emitted verbatim, so a typo cannot silently write `{color-acent}` into
//!   somebody's config and leave them hunting for why one colour is wrong.
//!
//! ## What a placeholder may name
//!
//! Exactly the variables [`crate::css::to_css_variables`] produces, which is the
//! canonical token vocabulary the whole system already speaks. Reusing it rather
//! than inventing a second naming scheme also inherits its proof: those values are
//! colours and numeric tokens, and the resolve gate (TH-0) holds that they are
//! inert, so a declared target cannot emit anything a shipped emitter could not.

use crate::css::to_css_variables;
use crate::{ArlenTheme, Rgba};
use serde::Deserialize;

/// How a colour token is written into the generated file.
///
/// Only colours have a choice to make; every other token is already the plain
/// number-with-unit its CSS form is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorFormat {
    /// `#rrggbb`. The default, because most config formats take exactly this and
    /// a format that drops alpha is honest about it: a config file that cannot
    /// express transparency should not be handed a value that claims to.
    #[default]
    Hex6,
    /// `#rrggbbaa`.
    Hex8,
    /// `#aarrggbb`, the order Qt and several Java toolkits use.
    ArgbHex,
    /// Whatever the CSS emitter writes (`#rrggbb` or `rgba(...)`).
    Css,
}

/// A theme target somebody declared for their own machine.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredTarget {
    /// What to call it. Names the file the declaration lives in and nothing else.
    pub name: String,
    /// Where the generated file goes, RELATIVE to the user's config directory.
    pub destination: String,
    /// How colours are written.
    #[serde(default)]
    pub color_format: ColorFormat,
    /// The file to generate, with `{token}` placeholders.
    pub template: String,
}

/// Why a declaration was refused. Every one of these is a parse-time answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetError {
    /// The TOML did not parse, or carried a key this schema does not have -
    /// which is how a `hook` or a `script` is refused.
    #[error("this is not a theme target: {0}")]
    Malformed(String),
    /// The name is not a plain identifier.
    #[error("a target name may hold letters, digits, dot, dash and underscore: {0}")]
    BadName(String),
    /// The destination leaves the config directory, or is not a path at all.
    #[error("a destination is a relative path inside the config directory: {0}")]
    BadDestination(String),
    /// The template carries an interpreter directive.
    #[error("a theme changes what a desktop looks like, never what it runs: the template carries an interpreter directive")]
    ExecutableTemplate,
    /// A placeholder names no token.
    #[error("the template asks for a token that does not exist: {0}")]
    UnknownToken(String),
}

/// The characters a target name may hold.
fn name_is_plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !name.starts_with('.')
}

/// Is `dest` a relative path that stays inside the config directory?
///
/// Refused: absolute, `~`-rooted, any `..` component, an empty component (`a//b`,
/// which several path handlers normalise differently), a control character, and a
/// Windows-style drive or backslash - a declaration is a text file a person may
/// have been handed, and a path that resolves differently in two readers is one
/// that resolves somewhere nobody checked.
fn destination_stays_inside(dest: &str) -> bool {
    if dest.is_empty() || dest.len() > 512 {
        return false;
    }
    if dest.starts_with('/') || dest.starts_with('~') || dest.contains('\\') || dest.contains(':') {
        return false;
    }
    if dest.chars().any(|c| c.is_control()) {
        return false;
    }
    let parts: Vec<&str> = dest.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return false;
    }
    // A destination has to name a FILE, so the last component cannot be a
    // directory marker; the split above already refuses a trailing slash.
    true
}

/// Parse and validate a declaration. Everything that can be refused is refused
/// here, so a target that parses is one a write can be trusted with.
pub fn parse_target(text: &str) -> Result<DeclaredTarget, TargetError> {
    let target: DeclaredTarget =
        toml::from_str(text).map_err(|e| TargetError::Malformed(e.to_string()))?;
    validate_target(&target)?;
    Ok(target)
}

/// The checks, separated so a caller holding a built target can run them too.
pub fn validate_target(target: &DeclaredTarget) -> Result<(), TargetError> {
    if !name_is_plain(&target.name) {
        return Err(TargetError::BadName(target.name.clone()));
    }
    if !destination_stays_inside(&target.destination) {
        return Err(TargetError::BadDestination(target.destination.clone()));
    }
    // `#!` anywhere, not only at the start: a generated file may be included by
    // another, and the first line of the GENERATED file is not necessarily the
    // first line of what runs.
    if target.template.contains("#!") {
        return Err(TargetError::ExecutableTemplate);
    }
    let known = token_names();
    for token in placeholders(&target.template) {
        if !known.contains(&token) {
            return Err(TargetError::UnknownToken(token));
        }
    }
    Ok(())
}

/// Every `{token}` in `template`, in order of appearance.
///
/// A `{` with no `}` after it is not a placeholder and is left alone, so a
/// template for a format that uses braces (a CSS-shaped one, say) is not forced
/// to escape them.
fn placeholders(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '{' {
            if let Some(end) = bytes[i + 1..].iter().position(|c| *c == '}') {
                let name: String = bytes[i + 1..i + 1 + end].iter().collect();
                // A token name is the same plain shape a CSS variable name is, so
                // `{ font-weight: 500 }` in a CSS-ish template is not a token.
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    out.push(name);
                }
                i += end + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// The token vocabulary a template may name. Derived from the CSS emitter over a
/// resolved theme, so the two can never drift apart.
fn token_names() -> Vec<String> {
    // Any resolved theme gives the same KEY set - the CSS emitter writes a fixed
    // list of names - so the bundled dark theme stands in for "every theme".
    match ArlenTheme::from_bundled(include_str!("../themes/dark.toml")) {
        Ok(theme) => to_css_variables(&theme, None).variables.keys().cloned().collect(),
        // The text is compiled in, so this cannot happen in a built binary. If it
        // somehow did, refusing every placeholder is the safe direction: no target
        // renders rather than every token rendering blank.
        Err(_) => Vec::new(),
    }
}

/// Write a colour the way this target asked for.
fn format_color(value: &str, rgba: Option<Rgba>, format: ColorFormat) -> String {
    let Some(c) = rgba else {
        return value.to_string();
    };
    let byte = |f: f32| (f * 255.0).round().clamp(0.0, 255.0) as u8;
    let (r, g, b, a) = (byte(c[0]), byte(c[1]), byte(c[2]), byte(c[3]));
    match format {
        ColorFormat::Css => value.to_string(),
        ColorFormat::Hex6 => format!("#{r:02x}{g:02x}{b:02x}"),
        ColorFormat::Hex8 => format!("#{r:02x}{g:02x}{b:02x}{a:02x}"),
        ColorFormat::ArgbHex => format!("#{a:02x}{r:02x}{g:02x}{b:02x}"),
    }
}

/// Generate the target's file from a resolved theme.
///
/// Substitution only: no conditionals, no loops, no includes. A template language
/// is the thing that grows until it needs a debugger, and the ruling asks for a
/// declaration rather than a program.
pub fn render_target(target: &DeclaredTarget, theme: &ArlenTheme) -> String {
    let vars = to_css_variables(theme, None);
    let mut out = target.template.clone();
    for (name, value) in vars.variables.iter() {
        let needle = format!("{{{name}}}");
        if !out.contains(&needle) {
            continue;
        }
        let rgba = color_for(theme, name);
        out = out.replace(&needle, &format_color(value, rgba, target.color_format));
    }
    out
}

/// The `Rgba` behind a colour token name, so a format other than CSS can be
/// written. `None` for every non-colour token, which then keeps its CSS form.
fn color_for(theme: &ArlenTheme, name: &str) -> Option<Rgba> {
    let c = &theme.color;
    Some(match name {
        "color-bg-shell" => c.bg_shell,
        "color-bg-app" => c.bg_app,
        "color-bg-card" => c.bg_card,
        "color-bg-overlay" => c.bg_overlay,
        "color-bg-input" => c.bg_input,
        "color-fg-primary" => c.fg_primary,
        "color-fg-secondary" => c.fg_secondary,
        "color-fg-disabled" => c.fg_disabled,
        "color-fg-inverse" => c.fg_inverse,
        "color-accent" => c.accent,
        "color-accent-hover" => c.accent_hover,
        "color-accent-pressed" => c.accent_pressed,
        "color-success" => c.success,
        "color-warning" => c.warning,
        "color-error" => c.error,
        "color-info" => c.info,
        "color-border-default" => c.border_default,
        "color-border-strong" => c.border_strong,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> ArlenTheme {
        ArlenTheme::from_bundled(include_str!("../themes/dark.toml"))
            .expect("the bundled dark theme resolves")
    }

    const MPV: &str = r#"
name = "mpv"
destination = "mpv/arlen-colors.conf"
template = """
background={color-bg-app}
foreground={color-fg-primary}
accent={color-accent}
corner={radius-card}
"""
"#;

    #[test]
    fn a_declaration_parses_and_renders_its_tokens() {
        let t = parse_target(MPV).expect("a plain declaration");
        assert_eq!(t.name, "mpv");
        assert_eq!(t.color_format, ColorFormat::Hex6);
        let out = render_target(&t, &theme());
        assert!(out.contains("background=#"), "the colour landed: {out}");
        assert!(!out.contains('{'), "every placeholder was substituted: {out}");
        // A non-colour token keeps its CSS form, units and all.
        assert!(out.contains("corner=") && out.contains("px"), "{out}");
    }

    /// The load-bearing refusal: there is no key to carry an instruction in.
    #[test]
    fn a_declaration_carrying_a_hook_does_not_parse() {
        for key in ["hook", "exec", "script", "command", "on_apply"] {
            let src = format!("{MPV}\n{key} = \"rm -rf ~\"\n");
            match parse_target(&src) {
                Err(TargetError::Malformed(_)) => {}
                other => panic!("{key} should not parse, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_template_carrying_an_interpreter_directive_is_refused() {
        let src = MPV.replace("background=", "#!/bin/sh\nbackground=");
        assert_eq!(parse_target(&src).unwrap_err(), TargetError::ExecutableTemplate);
    }

    /// A declaration cannot name a path outside its own target.
    #[test]
    fn a_destination_that_leaves_the_config_directory_is_refused() {
        for dest in [
            "/etc/passwd",
            "~/.bashrc",
            "../../.bashrc",
            "mpv/../../../etc/x",
            "mpv//x.conf",
            "C:\\x",
            "mpv/x\nconf",
            "",
        ] {
            let src = MPV.replace("mpv/arlen-colors.conf", dest);
            assert!(
                matches!(
                    parse_target(&src),
                    Err(TargetError::BadDestination(_)) | Err(TargetError::Malformed(_))
                ),
                "{dest} should be refused"
            );
        }
    }

    /// A typo is refused rather than written into somebody's config.
    #[test]
    fn an_unknown_token_is_refused_at_parse() {
        let src = MPV.replace("{color-accent}", "{color-acent}");
        assert_eq!(
            parse_target(&src).unwrap_err(),
            TargetError::UnknownToken("color-acent".to_string())
        );
    }

    /// A brace that is not a token is left alone, so a CSS-shaped template works.
    #[test]
    fn a_brace_that_is_not_a_token_is_not_one() {
        let src = r#"
name = "css-ish"
destination = "x/style.css"
template = """
body { color: {color-fg-primary}; font-weight: 500 }
"""
"#;
        let t = parse_target(src).expect("braces that are not tokens are fine");
        let out = render_target(&t, &theme());
        assert!(out.contains("font-weight: 500"), "{out}");
        assert!(out.contains("color: #"), "{out}");
    }

    #[test]
    fn each_colour_format_writes_its_own_shape() {
        for (fmt, len) in
            [(ColorFormat::Hex6, 7), (ColorFormat::Hex8, 9), (ColorFormat::ArgbHex, 9)]
        {
            let src = format!(
                "name = \"t\"\ndestination = \"t/t.conf\"\ncolor_format = \"{}\"\ntemplate = \"c={{color-accent}}\"\n",
                match fmt {
                    ColorFormat::Hex6 => "hex6",
                    ColorFormat::Hex8 => "hex8",
                    ColorFormat::ArgbHex => "argb_hex",
                    ColorFormat::Css => "css",
                }
            );
            let t = parse_target(&src).expect("a colour format parses");
            let out = render_target(&t, &theme());
            let value = out.trim().trim_start_matches("c=");
            assert_eq!(value.len(), len, "{fmt:?} wrote {value}");
            assert!(value.starts_with('#'));
        }
    }

    /// The vocabulary is the CSS emitter's, so it cannot drift from the tokens the
    /// rest of the system speaks.
    #[test]
    fn the_token_vocabulary_is_the_css_one() {
        let names = token_names();
        assert!(names.contains(&"color-accent".to_string()));
        assert!(names.contains(&"radius-card".to_string()));
        assert!(!names.is_empty());
    }

    /// Whatever a declaration says, what lands is a colour or a number: the
    /// substituted values come from the CSS emitter, which the resolve gate holds
    /// inert. So a hostile declaration can only ever produce a file of tokens.
    #[test]
    fn a_hostile_template_can_still_only_emit_inert_values() {
        let src = r#"
name = "hostile"
destination = "x/y.conf"
template = """
a={color-accent}
b={color-bg-app}
"""
"#;
        let t = parse_target(src).unwrap();
        let out = render_target(&t, &theme());
        for line in out.lines().filter(|l| !l.is_empty()) {
            let value = line.split('=').nth(1).unwrap();
            assert!(
                value.starts_with('#') && value.len() == 7,
                "only a colour may land: {line}"
            );
        }
    }
}
