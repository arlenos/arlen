/// Whether a failed command means the knowledge service is not running, rather
/// than that a read failed.
///
/// The backend answers with a marker token when the socket is absent (see
/// `src-tauri/src/service.rs`); the sentence lives in the pages, where it is
/// translated. Both ends name the same token, and the Rust side has a test
/// pinning it, because a rename on one side silently turns the honest sentence
/// back into the misleading one.
const NOT_RUNNING = "knowledge-daemon-not-running";

export function isServiceAbsent(err: unknown): boolean {
  return String(err).includes(NOT_RUNNING);
}
