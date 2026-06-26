//! CONN-R10 (§8.3): import the user's existing `~/.ssh/config` hosts as saved
//! connections, so "save once, use everywhere" inherits the hosts the user already
//! has. This is the pure parser - text to [`SshHost`] entries - used by the
//! Connections broker; reading the file, resolving `Include`/`Match`, and projecting
//! each host into a connection the Terminal (shell) and FM (SFTP) consume are the
//! layers on top.
//!
//! Scope (v1, honest): per-`Host`-block options (`HostName`/`User`/`Port`/
//! `IdentityFile`/`ProxyJump`) for concrete (non-wildcard) host aliases. A `Host *`
//! (or other wildcard pattern) is NOT emitted as a connection - it is an OpenSSH
//! default-merge construct, not a host you connect to - and per-host merging of a
//! wildcard block's defaults, plus `Include` and `Match`, are deliberate follow-ups
//! (noted, not silently dropped). Keys are matched case-insensitively per the
//! ssh_config(5) grammar; the FIRST value for a key within a block wins (OpenSSH's
//! first-match-wins semantics), so a duplicate key is ignored.

/// One imported SSH host: the alias the user types (`ssh <alias>`) plus the block's
/// resolved connection options. `None` fields were not set in the block (the broker
/// fills OpenSSH's own defaults - port 22, the login user - when it projects this).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshHost {
    /// The `Host` alias (one entry per concrete pattern on the `Host` line).
    pub alias: String,
    /// `HostName` - the real address to dial (absent means "the alias is the host").
    pub hostname: Option<String>,
    /// `User` - the login user.
    pub user: Option<String>,
    /// `Port` - the TCP port (a non-numeric or out-of-range value is ignored).
    pub port: Option<u16>,
    /// `IdentityFile` - the key path (the first one in the block).
    pub identity_file: Option<String>,
    /// `ProxyJump` - the jump host(s), carried through so a bastion connection
    /// projects intact.
    pub proxy_jump: Option<String>,
}

/// True for an ssh_config host pattern that is a wildcard/negation rather than a
/// concrete host: it configures defaults for matching hosts, so it is not itself a
/// connectable entry and is not imported.
fn is_wildcard_pattern(pattern: &str) -> bool {
    pattern.contains(['*', '?', '!'])
}

/// Strip a trailing `#` comment from a config line. ssh_config comments run to end
/// of line and are not honoured inside values, so a bare `#` (preceded by
/// whitespace or at line start) begins a comment; this keeps it simple and treats
/// any `#` as a comment start, which matches OpenSSH for the keys we read (none
/// take a value containing `#`).
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Split an ssh_config line into its keyword and the remainder (the value). The
/// grammar allows `Key value` or `Key = value`; tokens are whitespace-separated and
/// the optional `=` is dropped. Returns `None` for a blank line.
fn split_keyword(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (key, rest) = match trimmed.split_once(|c: char| c.is_whitespace() || c == '=') {
        Some((k, r)) => (k, r),
        None => (trimmed, ""),
    };
    if key.is_empty() {
        return None;
    }
    // The value may have a leading `=` (the `Key = value` form) and surrounding
    // whitespace; strip both.
    let value = rest.trim().trim_start_matches('=').trim();
    Some((key.to_ascii_lowercase(), value.to_string()))
}

/// Parse `~/.ssh/config` text into the concrete hosts it declares. Pure: takes the
/// file's contents, returns the imported [`SshHost`] entries in file order.
pub fn parse_ssh_config(text: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    // The block currently open: the indices into `hosts` for this `Host` line's
    // concrete patterns, so a following option line updates all of them.
    let mut current: Vec<usize> = Vec::new();

    for raw in text.lines() {
        let Some((key, value)) = split_keyword(strip_comment(raw)) else {
            continue;
        };
        if key == "host" {
            current.clear();
            for pattern in value.split_whitespace() {
                if is_wildcard_pattern(pattern) {
                    continue;
                }
                current.push(hosts.len());
                hosts.push(SshHost {
                    alias: pattern.to_string(),
                    ..SshHost::default()
                });
            }
            continue;
        }
        if value.is_empty() || current.is_empty() {
            continue;
        }
        for &i in &current {
            let host = &mut hosts[i];
            match key.as_str() {
                "hostname" if host.hostname.is_none() => host.hostname = Some(value.clone()),
                "user" if host.user.is_none() => host.user = Some(value.clone()),
                "port" if host.port.is_none() => host.port = value.parse::<u16>().ok(),
                "identityfile" if host.identity_file.is_none() => {
                    host.identity_file = Some(value.clone())
                }
                "proxyjump" if host.proxy_jump.is_none() => host.proxy_jump = Some(value.clone()),
                _ => {}
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_basic_host_block_imports_its_options() {
        let cfg = "Host work\n  HostName 10.0.0.5\n  User bob\n  Port 2222\n  IdentityFile ~/.ssh/work\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts.len(), 1);
        assert_eq!(
            hosts[0],
            SshHost {
                alias: "work".into(),
                hostname: Some("10.0.0.5".into()),
                user: Some("bob".into()),
                port: Some(2222),
                identity_file: Some("~/.ssh/work".into()),
                proxy_jump: None,
            }
        );
    }

    #[test]
    fn a_wildcard_host_is_not_imported() {
        let cfg = "Host *\n  User default\n\nHost real\n  HostName h\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts.iter().map(|h| h.alias.as_str()).collect::<Vec<_>>(), ["real"]);
        // The wildcard block's `User default` does not bleed onto `real` (v1: no
        // wildcard default-merge).
        assert_eq!(hosts[0].user, None);
    }

    #[test]
    fn multiple_patterns_on_one_host_line_each_get_the_block() {
        let cfg = "Host a b\n  User shared\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].alias, "a");
        assert_eq!(hosts[1].alias, "b");
        assert_eq!(hosts[0].user.as_deref(), Some("shared"));
        assert_eq!(hosts[1].user.as_deref(), Some("shared"));
    }

    #[test]
    fn proxy_jump_is_carried_through() {
        let cfg = "Host inner\n  HostName 10.1.1.1\n  ProxyJump bastion\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts[0].proxy_jump.as_deref(), Some("bastion"));
    }

    #[test]
    fn comments_blank_lines_and_the_equals_form_are_handled() {
        let cfg = "# a comment\n\nHost gw\n  HostName=192.168.1.1 # inline\n  Port = 22\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].hostname.as_deref(), Some("192.168.1.1"));
        assert_eq!(hosts[0].port, Some(22));
    }

    #[test]
    fn the_first_value_for_a_key_wins() {
        let cfg = "Host h\n  User first\n  User second\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts[0].user.as_deref(), Some("first"));
    }

    #[test]
    fn a_non_numeric_port_is_ignored() {
        let cfg = "Host h\n  Port nope\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts[0].port, None);
    }

    #[test]
    fn options_before_any_host_block_are_ignored() {
        // A stray option with no open Host block must not panic or attach anywhere.
        let cfg = "User orphan\nHost h\n  HostName x\n";
        let hosts = parse_ssh_config(cfg);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].user, None);
        assert_eq!(hosts[0].hostname.as_deref(), Some("x"));
    }
}
