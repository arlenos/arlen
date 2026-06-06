/// Keybinding config parser with scoped bindings.
///
/// Format: `~/.config/lunaris/keybindings.toml`
/// ```toml
/// [compositor]
/// "Super+Return" = "app:launch:terminal"
/// "Super+Q" = "window:close"
///
/// [shell]
/// "Super+Space" = "waypointer:toggle"
///
/// [titlebar]
/// "Ctrl+Tab" = "tab:next"
/// ```
///
/// See `docs/architecture/config-system.md` (Keybindings section).

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Keybinding parsing errors.
#[derive(Debug, Error)]
pub enum KeybindingError {
    #[error("unknown modifier: '{0}'")]
    UnknownModifier(String),
    #[error("empty key in binding: '{0}'")]
    EmptyKey(String),
    #[error("empty action in binding for '{0}'")]
    EmptyAction(String),
    #[error("invalid action format '{0}': expected category:action[:arg]")]
    InvalidAction(String),
    #[error("duplicate binding in [{scope}]: {key} (first: {first}, duplicate: {duplicate})")]
    Duplicate {
        scope: String,
        key: String,
        first: String,
        duplicate: String,
    },
    #[error("parse error: {0}")]
    ParseError(String),
}

// ---------------------------------------------------------------------------
// Modifiers
// ---------------------------------------------------------------------------

/// Modifier key flags (bitfield).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SUPER: Self = Self(1);
    pub const CTRL: Self = Self(2);
    pub const ALT: Self = Self(4);
    pub const SHIFT: Self = Self(8);

    pub fn has(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }

    fn set(&mut self, flag: Self) {
        self.0 |= flag.0;
    }
}

impl fmt::Display for Modifiers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.has(Self::SUPER) { parts.push("Super"); }
        if self.has(Self::CTRL) { parts.push("Ctrl"); }
        if self.has(Self::ALT) { parts.push("Alt"); }
        if self.has(Self::SHIFT) { parts.push("Shift"); }
        write!(f, "{}", parts.join("+"))
    }
}

/// Parse a single modifier name (case-insensitive, with aliases).
fn parse_modifier(s: &str) -> Result<Modifiers, KeybindingError> {
    match s.to_lowercase().as_str() {
        "super" | "meta" | "logo" | "mod4" => Ok(Modifiers::SUPER),
        "ctrl" | "control" => Ok(Modifiers::CTRL),
        "alt" | "mod1" => Ok(Modifiers::ALT),
        "shift" => Ok(Modifiers::SHIFT),
        _ => Err(KeybindingError::UnknownModifier(s.into())),
    }
}

// ---------------------------------------------------------------------------
// Keybinding
// ---------------------------------------------------------------------------

/// A parsed key binding: modifiers + key + action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybinding {
    pub modifiers: Modifiers,
    /// The non-modifier key (e.g. "Return", "q", "Space", "F4", "1").
    pub key: String,
    pub action: Action,
}

impl Keybinding {
    /// Canonical string representation for deduplication.
    /// Modifiers are always in Super+Ctrl+Alt+Shift order.
    pub fn canonical_key(&self) -> String {
        let mods = self.modifiers.to_string();
        if mods.is_empty() {
            self.key.clone()
        } else {
            format!("{mods}+{}", self.key)
        }
    }
}

/// Parse a key string like `"Super+Shift+Return"` into modifiers + key.
fn parse_key_string(raw: &str) -> Result<(Modifiers, String), KeybindingError> {
    let parts: Vec<&str> = raw.split('+').map(str::trim).collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(KeybindingError::EmptyKey(raw.into()));
    }

    let mut mods = Modifiers::NONE;
    let mut key_part = None;

    // Last non-modifier segment is the key. Iterate from the end.
    for (i, part) in parts.iter().enumerate().rev() {
        if i == parts.len() - 1 {
            // Last part: always the key unless it's a modifier-only binding.
            if parse_modifier(part).is_ok() && i > 0 {
                // Edge case: "Ctrl+Shift" with no key -- treat last as key anyway.
            }
            key_part = Some(part.to_string());
        } else {
            mods.set(parse_modifier(part)?);
        }
    }

    let key = key_part.ok_or_else(|| KeybindingError::EmptyKey(raw.into()))?;
    Ok((mods, key))
}

// ---------------------------------------------------------------------------
// Action
// ---------------------------------------------------------------------------

/// A parsed action: `category:action[:arg]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub category: String,
    pub action: String,
    pub arg: Option<String>,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.category, self.action)?;
        if let Some(ref a) = self.arg {
            write!(f, ":{a}")?;
        }
        Ok(())
    }
}

impl FromStr for Action {
    type Err = KeybindingError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        match parts.len() {
            2 => Ok(Action {
                category: parts[0].into(),
                action: parts[1].into(),
                arg: None,
            }),
            3 => Ok(Action {
                category: parts[0].into(),
                action: parts[1].into(),
                arg: Some(parts[2].into()),
            }),
            _ => Err(KeybindingError::InvalidAction(s.into())),
        }
    }
}

// ---------------------------------------------------------------------------
// Scope + Config
// ---------------------------------------------------------------------------

/// Binding scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    Compositor,
    Shell,
    Titlebar,
}

impl Scope {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "compositor" => Some(Self::Compositor),
            "shell" => Some(Self::Shell),
            "titlebar" => Some(Self::Titlebar),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Compositor => "compositor",
            Self::Shell => "shell",
            Self::Titlebar => "titlebar",
        }
    }
}

/// Parsed keybinding config with all scopes.
#[derive(Debug, Clone, Default)]
pub struct KeybindingConfig {
    pub compositor: Vec<Keybinding>,
    pub shell: Vec<Keybinding>,
    pub titlebar: Vec<Keybinding>,
}

impl KeybindingConfig {
    /// Get bindings for a scope.
    pub fn for_scope(&self, scope: Scope) -> &[Keybinding] {
        match scope {
            Scope::Compositor => &self.compositor,
            Scope::Shell => &self.shell,
            Scope::Titlebar => &self.titlebar,
        }
    }

    /// Find the action for a key combo in a scope.
    pub fn lookup(&self, scope: Scope, modifiers: Modifiers, key: &str) -> Option<&Action> {
        self.for_scope(scope)
            .iter()
            .find(|b| b.modifiers == modifiers && b.key == key)
            .map(|b| &b.action)
    }
}

/// Parse a keybindings TOML string into a `KeybindingConfig`.
///
/// Returns the config and a list of warnings (duplicates within scope).
pub fn parse_keybindings(content: &str) -> Result<(KeybindingConfig, Vec<KeybindingError>), KeybindingError> {
    let table: toml::Value =
        toml::from_str(content).map_err(|e| KeybindingError::ParseError(e.to_string()))?;

    let root = table
        .as_table()
        .ok_or_else(|| KeybindingError::ParseError("expected TOML table".into()))?;

    let mut config = KeybindingConfig::default();
    let mut warnings = Vec::new();

    for (scope_name, section) in root {
        let scope = match Scope::from_str(scope_name) {
            Some(s) => s,
            None => continue, // ignore unknown sections
        };

        let bindings_table = match section.as_table() {
            Some(t) => t,
            None => continue,
        };

        let mut seen: HashMap<String, String> = HashMap::new();

        for (key_str, action_val) in bindings_table {
            let action_str = match action_val.as_str() {
                Some(s) => s,
                None => continue,
            };

            let (mods, key) = parse_key_string(key_str)?;

            if action_str.is_empty() {
                return Err(KeybindingError::EmptyAction(key_str.clone()));
            }

            let action: Action = action_str.parse()?;

            let binding = Keybinding {
                modifiers: mods,
                key,
                action,
            };

            let canonical = binding.canonical_key();

            if let Some(first_action) = seen.get(&canonical) {
                warnings.push(KeybindingError::Duplicate {
                    scope: scope.as_str().into(),
                    key: canonical.clone(),
                    first: first_action.clone(),
                    duplicate: action_str.into(),
                });
                continue; // first wins
            }

            seen.insert(canonical, action_str.into());

            match scope {
                Scope::Compositor => config.compositor.push(binding),
                Scope::Shell => config.shell.push(binding),
                Scope::Titlebar => config.titlebar.push(binding),
            }
        }
    }

    Ok((config, warnings))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Modifier parsing ──

    #[test]
    fn test_parse_modifier_aliases() {
        assert_eq!(parse_modifier("Super").unwrap(), Modifiers::SUPER);
        assert_eq!(parse_modifier("meta").unwrap(), Modifiers::SUPER);
        assert_eq!(parse_modifier("Logo").unwrap(), Modifiers::SUPER);
        assert_eq!(parse_modifier("Ctrl").unwrap(), Modifiers::CTRL);
        assert_eq!(parse_modifier("Control").unwrap(), Modifiers::CTRL);
        assert_eq!(parse_modifier("Alt").unwrap(), Modifiers::ALT);
        assert_eq!(parse_modifier("Shift").unwrap(), Modifiers::SHIFT);
        assert!(parse_modifier("Hyper").is_err());
    }

    // ── Key string parsing ──

    #[test]
    fn test_parse_key_simple() {
        let (mods, key) = parse_key_string("Super+Return").unwrap();
        assert!(mods.has(Modifiers::SUPER));
        assert_eq!(key, "Return");
    }

    #[test]
    fn test_parse_key_multi_modifier() {
        let (mods, key) = parse_key_string("Ctrl+Shift+A").unwrap();
        assert!(mods.has(Modifiers::CTRL));
        assert!(mods.has(Modifiers::SHIFT));
        assert!(!mods.has(Modifiers::SUPER));
        assert_eq!(key, "A");
    }

    #[test]
    fn test_parse_key_no_modifier() {
        let (mods, key) = parse_key_string("F11").unwrap();
        assert_eq!(mods, Modifiers::NONE);
        assert_eq!(key, "F11");
    }

    #[test]
    fn test_modifier_order_irrelevant() {
        let (m1, k1) = parse_key_string("Ctrl+Shift+A").unwrap();
        let (m2, k2) = parse_key_string("Shift+Ctrl+A").unwrap();
        assert_eq!(m1, m2);
        assert_eq!(k1, k2);
    }

    // ── Action parsing ──

    #[test]
    fn test_action_two_parts() {
        let a: Action = "window:close".parse().unwrap();
        assert_eq!(a.category, "window");
        assert_eq!(a.action, "close");
        assert!(a.arg.is_none());
    }

    #[test]
    fn test_action_three_parts() {
        let a: Action = "app:launch:terminal".parse().unwrap();
        assert_eq!(a.category, "app");
        assert_eq!(a.action, "launch");
        assert_eq!(a.arg.as_deref(), Some("terminal"));
    }

    #[test]
    fn test_action_display() {
        let a: Action = "workspace:goto:3".parse().unwrap();
        assert_eq!(a.to_string(), "workspace:goto:3");

        let b: Action = "window:close".parse().unwrap();
        assert_eq!(b.to_string(), "window:close");
    }

    #[test]
    fn test_action_invalid() {
        assert!("onlyonepart".parse::<Action>().is_err());
    }

    // ── Full config parsing ──

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[compositor]
"Super+Return" = "app:launch:terminal"
"Super+Q" = "window:close"
"Super+F" = "window:fullscreen"

[shell]
"Super+Space" = "waypointer:toggle"

[titlebar]
"Ctrl+Tab" = "tab:next"
"Ctrl+Shift+Tab" = "tab:prev"
"#;
        let (config, warnings) = parse_keybindings(toml).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(config.compositor.len(), 3);
        assert_eq!(config.shell.len(), 1);
        assert_eq!(config.titlebar.len(), 2);
    }

    #[test]
    fn test_lookup() {
        let toml = r#"
[compositor]
"Super+Q" = "window:close"
"#;
        let (config, _) = parse_keybindings(toml).unwrap();
        let action = config.lookup(Scope::Compositor, Modifiers::SUPER, "Q");
        assert_eq!(action.unwrap().action, "close");

        assert!(config.lookup(Scope::Shell, Modifiers::SUPER, "Q").is_none());
    }

    // ── Duplicate detection ──

    #[test]
    fn test_duplicate_different_format() {
        // TOML rejects identical keys, but different formats for the same
        // key combo should be detected (e.g. Ctrl+Shift+A vs Shift+Ctrl+A).
        let toml = r#"
[compositor]
"Ctrl+Shift+A" = "window:close"
"Shift+Ctrl+A" = "window:minimize"
"#;
        let (config, warnings) = parse_keybindings(toml).unwrap();
        // First wins, duplicate warned.
        assert_eq!(config.compositor.len(), 1);
        assert_eq!(config.compositor[0].action.action, "close");
        assert_eq!(warnings.len(), 1);
        assert!(matches!(&warnings[0], KeybindingError::Duplicate { .. }));
    }

    #[test]
    fn test_same_key_different_scopes_ok() {
        let toml = r#"
[compositor]
"Super+Q" = "window:close"

[shell]
"Super+Q" = "shell:quit"
"#;
        let (config, warnings) = parse_keybindings(toml).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(config.compositor.len(), 1);
        assert_eq!(config.shell.len(), 1);
    }

    // ── Error cases ──

    #[test]
    fn test_unknown_modifier_error() {
        let toml = r#"
[compositor]
"Hyper+Q" = "window:close"
"#;
        assert!(parse_keybindings(toml).is_err());
    }

    #[test]
    fn test_empty_action_error() {
        let toml = r#"
[compositor]
"Super+Q" = ""
"#;
        assert!(parse_keybindings(toml).is_err());
    }

    #[test]
    fn test_canonical_key_normalized() {
        let b1 = Keybinding {
            modifiers: {
                let mut m = Modifiers::NONE;
                m.set(Modifiers::CTRL);
                m.set(Modifiers::SHIFT);
                m
            },
            key: "A".into(),
            action: "test:action".parse().unwrap(),
        };
        // Canonical always puts modifiers in Super+Ctrl+Alt+Shift order.
        assert_eq!(b1.canonical_key(), "Ctrl+Shift+A");
    }

    #[test]
    fn test_unknown_scope_ignored() {
        let toml = r#"
[unknown_scope]
"Super+Q" = "some:action"

[compositor]
"Super+Return" = "app:launch:terminal"
"#;
        let (config, warnings) = parse_keybindings(toml).unwrap();
        assert!(warnings.is_empty());
        assert_eq!(config.compositor.len(), 1);
    }
}
