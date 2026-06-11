//! The rclone rc JSON-RPC client (online-accounts-plan.md OA-R3).
//!
//! For an account's `Files` service the daemon drives a CONFINED rclone (its own
//! `arlen-run` subprocess: Landlock + a per-host egress allowlist scoped to the
//! provider + a cgroup) over rclone's remote-control API: a POST of a method path
//! (`mount/mount`, `vfs/refresh`, `core/version`, ...) with a JSON params body,
//! returning JSON. This module is the typed protocol over a [`RcTransport`] seam,
//! so the method layer is tested with a mock; the real HTTP-over-socket transport
//! and the rclone subprocess management are the on-kernel integration on top.

use serde::Deserialize;

/// An error driving rclone over the rc API.
#[derive(Debug, thiserror::Error)]
pub enum RcError {
    /// The transport (socket / HTTP) failed.
    #[error("transport: {0}")]
    Transport(String),
    /// rclone returned an error result (`{error, status}` on an HTTP 4xx/5xx).
    #[error("rclone error ({status}): {message}")]
    Rclone {
        /// The rc HTTP status.
        status: u16,
        /// The error message rclone reported.
        message: String,
    },
    /// The response was not the shape the method expected.
    #[error("unexpected response: {0}")]
    Unexpected(String),
}

/// The transport that carries one rc method call: a POST of `path` with the JSON
/// `params`, yielding the decoded JSON result (or [`RcError::Rclone`] for an rc
/// error result, [`RcError::Transport`] for a socket failure). The real impl is
/// an HTTP client over the confined rclone's Unix socket; tests use a mock.
#[async_trait::async_trait]
pub trait RcTransport: Send + Sync {
    /// Call rc method `path` with `params`; return the decoded result JSON.
    async fn call(
        &self,
        path: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RcError>;
}

/// One active FUSE mount, as reported by `mount/listmounts`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MountInfo {
    /// The remote/fs string (`gdrive:`).
    #[serde(rename = "Fs")]
    pub fs: String,
    /// The local mount point.
    #[serde(rename = "MountPoint")]
    pub mount_point: String,
}

/// The typed rc client over a transport.
pub struct RcClient<T> {
    transport: T,
}

impl<T: RcTransport> RcClient<T> {
    /// A client driving rclone over `transport`.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// `core/version` - a health probe; returns the rclone version string.
    pub async fn version(&self) -> Result<String, RcError> {
        let resp = self
            .transport
            .call("core/version", serde_json::json!({}))
            .await?;
        resp.get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| RcError::Unexpected("no version field".into()))
    }

    /// `mount/mount` - FUSE-mount the remote `fs` at `mount_point` (the realised
    /// `Files` capability: the cloud drive becomes a local mount).
    pub async fn mount(&self, fs: &str, mount_point: &str) -> Result<(), RcError> {
        self.transport
            .call(
                "mount/mount",
                serde_json::json!({ "fs": fs, "mountPoint": mount_point }),
            )
            .await
            .map(|_| ())
    }

    /// `mount/unmount` - unmount `mount_point`.
    pub async fn unmount(&self, mount_point: &str) -> Result<(), RcError> {
        self.transport
            .call(
                "mount/unmount",
                serde_json::json!({ "mountPoint": mount_point }),
            )
            .await
            .map(|_| ())
    }

    /// `mount/listmounts` - the active FUSE mounts.
    pub async fn list_mounts(&self) -> Result<Vec<MountInfo>, RcError> {
        let resp = self
            .transport
            .call("mount/listmounts", serde_json::json!({}))
            .await?;
        let points = resp
            .get("mountPoints")
            .ok_or_else(|| RcError::Unexpected("no mountPoints field".into()))?;
        serde_json::from_value(points.clone())
            .map_err(|e| RcError::Unexpected(format!("mountPoints: {e}")))
    }

    /// `vfs/refresh` - refresh the VFS directory cache (so a change made out of
    /// band shows in the mount).
    pub async fn vfs_refresh(&self) -> Result<(), RcError> {
        self.transport
            .call("vfs/refresh", serde_json::json!({}))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A transport that returns a canned result per path, or a canned rc error,
    /// and records the params it was called with.
    #[derive(Default)]
    struct MockTransport {
        results: HashMap<String, serde_json::Value>,
        error: Option<(u16, String)>,
        calls: Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait::async_trait]
    impl RcTransport for MockTransport {
        async fn call(
            &self,
            path: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, RcError> {
            self.calls
                .lock()
                .unwrap()
                .push((path.to_string(), params.clone()));
            if let Some((status, message)) = &self.error {
                return Err(RcError::Rclone {
                    status: *status,
                    message: message.clone(),
                });
            }
            Ok(self.results.get(path).cloned().unwrap_or(serde_json::json!({})))
        }
    }

    #[tokio::test]
    async fn version_extracts_the_version_field() {
        let mut t = MockTransport::default();
        t.results.insert(
            "core/version".into(),
            serde_json::json!({ "version": "v1.66.0" }),
        );
        let rc = RcClient::new(t);
        assert_eq!(rc.version().await.unwrap(), "v1.66.0");
    }

    #[tokio::test]
    async fn mount_posts_the_fs_and_mountpoint() {
        let t = MockTransport::default();
        let rc = RcClient::new(t);
        rc.mount("gdrive:", "/home/x/Drive").await.unwrap();
        let calls = rc.transport.calls.lock().unwrap();
        assert_eq!(calls[0].0, "mount/mount");
        assert_eq!(calls[0].1["fs"], "gdrive:");
        assert_eq!(calls[0].1["mountPoint"], "/home/x/Drive");
    }

    #[tokio::test]
    async fn list_mounts_parses_the_pascal_case_fields() {
        let mut t = MockTransport::default();
        t.results.insert(
            "mount/listmounts".into(),
            serde_json::json!({
                "mountPoints": [
                    { "Fs": "gdrive:", "MountPoint": "/home/x/Drive", "MountedOn": "2026-06-11T00:00:00Z" }
                ]
            }),
        );
        let rc = RcClient::new(t);
        let mounts = rc.list_mounts().await.unwrap();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].fs, "gdrive:");
        assert_eq!(mounts[0].mount_point, "/home/x/Drive");
    }

    #[tokio::test]
    async fn an_rc_error_propagates() {
        let t = MockTransport {
            error: Some((500, "directory not found".into())),
            ..Default::default()
        };
        let rc = RcClient::new(t);
        let err = rc.mount("gdrive:", "/bad").await.unwrap_err();
        assert!(matches!(err, RcError::Rclone { status: 500, .. }), "got {err:?}");
    }
}
