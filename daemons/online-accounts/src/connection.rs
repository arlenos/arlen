//! CONN-R9/R10 (§8.2/§8.4): the saved-connection descriptor + the pure pipeline that
//! turns an imported SSH host (or a configured backend) into the rclone inline
//! connection string a mount uses. This unifies the two connections cores: an
//! [`SshHost`](crate::ssh_config::SshHost) projects into a [`SavedConnection`], and a
//! [`SavedConnection`] renders into the inline `fs` string via
//! [`rclone_connection_string`](crate::rc::rclone_connection_string).
//!
//! The descriptor holds ONLY non-secret endpoint intent (§8.1): the credential is
//! injected at render time from the broker, never stored here. The rclone password
//! `obscure` form is the caller's concern (the daemon obscures before passing it as
//! `secret`); this layer owns the field mapping + ordering, the connection-string
//! builder owns the injection-safe quoting. Reading the config + spawning the
//! confined rclone are the on-kernel layers on top.

use crate::rc::rclone_connection_string;
use crate::ssh_config::SshHost;

/// A mountable backend kind, mapping to the rclone backend of the same breadth. The
/// §8.2 coverage grows here as each backend's parameter mapping is added; today the
/// SSH-is-also-SFTP path (`sftp`) plus `ftp` and `webdav` are mapped (their rclone
/// parameter names verified against `rclone help backend`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionBackend {
    /// SSH/SFTP (`sftp`): `host`/`user`/`port`/`key_file`/`pass`.
    Sftp,
    /// FTP/FTPS (`ftp`): `host`/`user`/`port`/`pass`.
    Ftp,
    /// WebDAV (`webdav`): `url`/`user`/`pass`.
    Webdav,
}

impl ConnectionBackend {
    /// The rclone backend name this maps to.
    fn rclone_name(self) -> &'static str {
        match self {
            ConnectionBackend::Sftp => "sftp",
            ConnectionBackend::Ftp => "ftp",
            ConnectionBackend::Webdav => "webdav",
        }
    }
}

/// A saved connection's endpoint intent (no secret; §8.1). `path` is the remote path
/// within the backend (empty for the mount root). Field use is per backend: `sftp`/
/// `ftp` use `host`/`port`/`user` (and `sftp` also `key_file`); `webdav` uses `url`/
/// `user`. The secret (a password, or an SFTP key passphrase) is supplied at render
/// time, never stored on the descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedConnection {
    /// Stable id (the SSH alias, or a configured connection's id).
    pub id: String,
    /// Which backend this connection mounts through.
    pub backend: ConnectionBackend,
    /// The host to dial (`sftp`/`ftp`).
    pub host: Option<String>,
    /// The TCP port (`sftp`/`ftp`); rclone defaults it when absent.
    pub port: Option<u16>,
    /// The login user.
    pub user: Option<String>,
    /// The SFTP private-key path (the imported `IdentityFile`).
    pub key_file: Option<String>,
    /// The WebDAV endpoint URL.
    pub url: Option<String>,
    /// The remote path within the backend (empty for the mount root).
    pub path: String,
}

/// Project an imported SSH host into a saved SFTP connection (§8.3: SSH is also
/// SFTP). The alias is the id and the dialed host falls back to the alias when no
/// `HostName` was set (ssh's own behaviour); the `IdentityFile` becomes the SFTP
/// key. No secret is carried - key-file auth uses the file, password auth injects
/// `pass` at render time. `ProxyJump` is intentionally not mapped: rclone's sftp
/// backend has no jump-host parameter, so a bastion is carried at the SSH transport
/// layer (ControlMaster), not in the rclone descriptor.
pub fn from_ssh_host(host: &SshHost) -> SavedConnection {
    SavedConnection {
        id: host.alias.clone(),
        backend: ConnectionBackend::Sftp,
        host: Some(host.hostname.clone().unwrap_or_else(|| host.alias.clone())),
        port: host.port,
        user: host.user.clone(),
        key_file: host.identity_file.clone(),
        url: None,
        path: String::new(),
    }
}

impl SavedConnection {
    /// The rclone parameters for this connection, in a fixed deterministic order,
    /// with the optional `secret` injected as the backend's password parameter. Only
    /// the fields the backend uses are emitted; an absent optional field is omitted
    /// so rclone applies its own default.
    fn rclone_params(&self, secret: Option<&str>) -> Vec<(String, String)> {
        let mut params: Vec<(String, String)> = Vec::new();
        let mut push = |k: &str, v: &str| params.push((k.to_string(), v.to_string()));
        match self.backend {
            ConnectionBackend::Sftp | ConnectionBackend::Ftp => {
                if let Some(h) = &self.host {
                    push("host", h);
                }
                if let Some(u) = &self.user {
                    push("user", u);
                }
                if let Some(p) = self.port {
                    push("port", &p.to_string());
                }
                if matches!(self.backend, ConnectionBackend::Sftp) {
                    if let Some(k) = &self.key_file {
                        push("key_file", k);
                    }
                }
            }
            ConnectionBackend::Webdav => {
                if let Some(url) = &self.url {
                    push("url", url);
                }
                if let Some(u) = &self.user {
                    push("user", u);
                }
            }
        }
        if let Some(s) = secret {
            push("pass", s);
        }
        params
    }

    /// Render this connection as an rclone inline connection string `fs`, with the
    /// broker-supplied `secret` injected as `pass` (§8.1: the secret reaches rclone
    /// only inline at mount time, never its on-disk config). `secret` is `None` for
    /// key-file auth. The value quoting that prevents a credential/host breaking out
    /// of its parameter slot is the builder's job.
    pub fn to_connection_string(&self, secret: Option<&str>) -> String {
        let owned = self.rclone_params(secret);
        let refs: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        rclone_connection_string(self.backend.rclone_name(), &refs, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_config::SshHost;

    #[test]
    fn an_ssh_host_projects_to_an_sftp_connection() {
        let host = SshHost {
            alias: "work".into(),
            hostname: Some("10.0.0.5".into()),
            user: Some("bob".into()),
            port: Some(2222),
            identity_file: Some("~/.ssh/work".into()),
            proxy_jump: Some("bastion".into()),
        };
        let c = from_ssh_host(&host);
        assert_eq!(c.backend, ConnectionBackend::Sftp);
        assert_eq!(c.host.as_deref(), Some("10.0.0.5"));
        assert_eq!(c.user.as_deref(), Some("bob"));
        assert_eq!(c.port, Some(2222));
        assert_eq!(c.key_file.as_deref(), Some("~/.ssh/work"));
    }

    #[test]
    fn a_host_without_hostname_dials_its_alias() {
        let host = SshHost {
            alias: "myhost".into(),
            ..SshHost::default()
        };
        assert_eq!(from_ssh_host(&host).host.as_deref(), Some("myhost"));
    }

    #[test]
    fn a_key_file_sftp_connection_has_no_pass() {
        let c = from_ssh_host(&SshHost {
            alias: "h".into(),
            hostname: Some("host".into()),
            user: Some("u".into()),
            port: Some(22),
            identity_file: Some("/k".into()),
            proxy_jump: None,
        });
        assert_eq!(c.to_connection_string(None), ":sftp,host=host,user=u,port=22,key_file=/k:");
    }

    #[test]
    fn a_password_secret_is_injected_as_pass() {
        let c = SavedConnection {
            id: "h".into(),
            backend: ConnectionBackend::Sftp,
            host: Some("host".into()),
            port: None,
            user: Some("u".into()),
            key_file: None,
            url: None,
            path: String::new(),
        };
        // OBSCURED stands for the already-obscured form the daemon supplies.
        assert_eq!(c.to_connection_string(Some("OBSCURED")), ":sftp,host=host,user=u,pass=OBSCURED:");
    }

    #[test]
    fn webdav_uses_url_and_user() {
        let c = SavedConnection {
            id: "dav".into(),
            backend: ConnectionBackend::Webdav,
            host: None,
            port: None,
            user: Some("u".into()),
            key_file: None,
            url: Some("https://dav.example/remote.php".into()),
            path: "files".into(),
        };
        // The URL contains `:` (in `https://`), so the builder quotes it whole - it
        // would otherwise read as the path-section separator.
        assert_eq!(
            c.to_connection_string(Some("pw")),
            ":webdav,url=\"https://dav.example/remote.php\",user=u,pass=pw:files"
        );
    }

    #[test]
    fn ftp_omits_the_key_file_and_uses_host_port() {
        let c = SavedConnection {
            id: "f".into(),
            backend: ConnectionBackend::Ftp,
            host: Some("ftp.host".into()),
            port: Some(21),
            user: Some("u".into()),
            key_file: Some("/ignored".into()),
            url: None,
            path: String::new(),
        };
        assert_eq!(c.to_connection_string(None), ":ftp,host=ftp.host,user=u,port=21:");
    }
}
