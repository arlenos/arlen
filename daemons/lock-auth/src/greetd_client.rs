//! The greetd auth-conversation client for the GREETER surface
//! (greeter-onboarding-plan.md GR-R1: "use greetd + the greetd_ipc crate, do not
//! reprogram auth"). Feature-gated behind `greetd` and exercised on hardware: it
//! speaks the greetd JSON protocol over the `GREETD_SOCK` Unix socket.
//!
//! Why this differs from [`crate::pam_verifier`]: the LOCK SCREEN runs the PAM
//! conversation itself (it is re-authing a live session), so it uses
//! [`crate::pam_verifier::PamVerifier`]; the GREETER does NOT - greetd owns the
//! PAM conversation (and the `pam_systemd_home` key release), and the greeter is
//! a thin client relaying greetd's prompts to the UI and the user's answers
//! back. So this is a conversation state machine, not a one-shot
//! [`crate::auth::FactorVerifier`]: greetd's auth flow is interactive (it may ask
//! a password, then a TOTP, then an info message), and the protocol's own
//! guidance is to make NO assumption about the questions and never auto-answer
//! them. The UI (arlen-ui) drives the loop, rendering each [`AuthStep`] and
//! posting the user's response; the backend here is just the typed protocol
//! driver, which is why it is generic over the stream and tested against a
//! scripted greetd rather than a live socket.
//!
//! Connecting greetd's result to the shared core: on [`AuthStep::Authenticated`]
//! greetd has verified the password and released the home key, so the greeter
//! treats it as a verified [`crate::tier::Factor::Password`] on a cold session
//! and runs it through the shared tier evaluation + [`crate::audit`] for one
//! consistent unlock record across both surfaces.

#![cfg(feature = "greetd")]

use std::io::{Read, Write};

use greetd_ipc::codec::SyncCodec;
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};

/// The environment variable greetd sets to the path of its IPC socket for the
/// greeter process.
pub const GREETD_SOCK_ENV: &str = "GREETD_SOCK";

/// One step in the greetd auth conversation, surfaced for the UI to render.
/// The caller answers a [`AuthStep::Prompt`] (and acknowledges a
/// [`AuthStep::Message`]) with [`GreetdClient::post_response`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStep {
    /// greetd asks a question. `secret` is true when the answer must be hidden
    /// (a password / PIN); false for a visible answer (e.g. an echoed prompt).
    Prompt {
        /// Whether the answer should be hidden during input.
        secret: bool,
        /// The prompt text to display verbatim (no assumption about its meaning).
        text: String,
    },
    /// An informational or error message to display. The conversation continues:
    /// the caller still posts the next response (`None` to just acknowledge).
    Message {
        /// Whether this is an error message rather than an info message.
        error: bool,
        /// The message text.
        text: String,
    },
    /// Authentication succeeded. The session may now be started with
    /// [`GreetdClient::start_session`]. For the greeter (a cold boot) this is the
    /// key-release moment greetd performed via `pam_systemd_home`.
    Authenticated,
    /// Authentication failed (greetd reported [`ErrorType::AuthError`]); the
    /// credential was wrong. The caller may retry or [`GreetdClient::cancel`].
    Failed {
        /// greetd's human-readable failure description.
        description: String,
    },
}

/// A greetd protocol or transport failure (distinct from an auth *failure*,
/// which is the well-formed [`AuthStep::Failed`]).
#[derive(Debug)]
pub enum GreetdError {
    /// A transport / codec error talking to greetd.
    Io(String),
    /// greetd returned a generic (non-auth) error, or a response that does not
    /// belong at this point in the flow.
    Protocol(String),
}

impl std::fmt::Display for GreetdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GreetdError::Io(e) => write!(f, "greetd transport: {e}"),
            GreetdError::Protocol(e) => write!(f, "greetd protocol: {e}"),
        }
    }
}

impl std::error::Error for GreetdError {}

impl From<greetd_ipc::codec::Error> for GreetdError {
    fn from(e: greetd_ipc::codec::Error) -> Self {
        GreetdError::Io(e.to_string())
    }
}

/// A typed driver for one greetd auth conversation over `stream`.
///
/// Generic over the stream so the state machine is tested against a scripted
/// greetd; in production the stream is the `GREETD_SOCK` `UnixStream`.
pub struct GreetdClient<S> {
    stream: S,
}

impl<S: Read + Write> GreetdClient<S> {
    /// Wrap a connected greetd stream.
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Begin a login attempt for `username`, returning greetd's first step.
    pub fn create_session(&mut self, username: &str) -> Result<AuthStep, GreetdError> {
        Request::CreateSession {
            username: username.to_string(),
        }
        .write_to(&mut self.stream)?;
        self.read_step()
    }

    /// Answer the last [`AuthStep::Prompt`] (or acknowledge a
    /// [`AuthStep::Message`] with `None`) and return greetd's next step.
    pub fn post_response(&mut self, response: Option<String>) -> Result<AuthStep, GreetdError> {
        Request::PostAuthMessageResponse { response }.write_to(&mut self.stream)?;
        self.read_step()
    }

    /// Start the session after [`AuthStep::Authenticated`]. `cmd` is the session
    /// command + args; `env` is extra environment entries.
    pub fn start_session(&mut self, cmd: Vec<String>, env: Vec<String>) -> Result<(), GreetdError> {
        Request::StartSession { cmd, env }.write_to(&mut self.stream)?;
        match Response::read_from(&mut self.stream)? {
            Response::Success => Ok(()),
            Response::Error {
                error_type,
                description,
            } => Err(GreetdError::Protocol(format!(
                "start_session refused ({error_type:?}): {description}"
            ))),
            Response::AuthMessage { .. } => Err(GreetdError::Protocol(
                "greetd asked for auth during start_session".to_string(),
            )),
        }
    }

    /// Cancel the in-flight session (before it is started).
    pub fn cancel(&mut self) -> Result<(), GreetdError> {
        Request::CancelSession.write_to(&mut self.stream)?;
        match Response::read_from(&mut self.stream)? {
            Response::Success => Ok(()),
            Response::Error { description, .. } => Err(GreetdError::Protocol(description)),
            Response::AuthMessage { .. } => Err(GreetdError::Protocol(
                "greetd asked for auth during cancel".to_string(),
            )),
        }
    }

    /// Read and classify the next greetd response into an [`AuthStep`].
    fn read_step(&mut self) -> Result<AuthStep, GreetdError> {
        match Response::read_from(&mut self.stream)? {
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => Ok(match auth_message_type {
                AuthMessageType::Secret => AuthStep::Prompt {
                    secret: true,
                    text: auth_message,
                },
                AuthMessageType::Visible => AuthStep::Prompt {
                    secret: false,
                    text: auth_message,
                },
                AuthMessageType::Info => AuthStep::Message {
                    error: false,
                    text: auth_message,
                },
                AuthMessageType::Error => AuthStep::Message {
                    error: true,
                    text: auth_message,
                },
            }),
            Response::Success => Ok(AuthStep::Authenticated),
            // An auth failure is a well-formed conversation outcome, not a
            // protocol error: surface it as Failed so the UI can offer a retry.
            Response::Error {
                error_type: ErrorType::AuthError,
                description,
            } => Ok(AuthStep::Failed { description }),
            // A generic error is a real protocol/setup fault.
            Response::Error {
                error_type: ErrorType::Error,
                description,
            } => Err(GreetdError::Protocol(description)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A scripted greetd: serves pre-written [`Response`]s in order and records
    /// the [`Request`]s the client writes, so the conversation state machine is
    /// tested without a live greetd.
    struct ScriptedGreetd {
        to_read: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl ScriptedGreetd {
        fn new(responses: Vec<Response>) -> Self {
            let mut buf = Vec::new();
            for r in &responses {
                r.write_to(&mut buf).expect("serialise scripted response");
            }
            Self {
                to_read: Cursor::new(buf),
                written: Vec::new(),
            }
        }
    }

    impl Read for ScriptedGreetd {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.to_read.read(buf)
        }
    }

    impl Write for ScriptedGreetd {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_secret_prompt_then_success_drives_a_password_login() {
        let mut client = GreetdClient::new(ScriptedGreetd::new(vec![
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password:".to_string(),
            },
            Response::Success,
        ]));
        let step = client.create_session("alice").expect("create session");
        assert_eq!(
            step,
            AuthStep::Prompt {
                secret: true,
                text: "Password:".to_string()
            }
        );
        let step = client
            .post_response(Some("hunter2".to_string()))
            .expect("post password");
        assert_eq!(step, AuthStep::Authenticated);
    }

    #[test]
    fn a_wrong_password_surfaces_as_failed_not_a_protocol_error() {
        let mut client = GreetdClient::new(ScriptedGreetd::new(vec![
            Response::AuthMessage {
                auth_message_type: AuthMessageType::Secret,
                auth_message: "Password:".to_string(),
            },
            Response::Error {
                error_type: ErrorType::AuthError,
                description: "authentication failed".to_string(),
            },
        ]));
        client.create_session("alice").expect("create session");
        let step = client.post_response(Some("wrong".to_string())).expect("post");
        assert_eq!(
            step,
            AuthStep::Failed {
                description: "authentication failed".to_string()
            }
        );
    }

    #[test]
    fn an_info_message_is_surfaced_and_the_conversation_continues() {
        let mut client = GreetdClient::new(ScriptedGreetd::new(vec![Response::AuthMessage {
            auth_message_type: AuthMessageType::Info,
            auth_message: "Insert your token".to_string(),
        }]));
        let step = client.create_session("alice").expect("create session");
        assert_eq!(
            step,
            AuthStep::Message {
                error: false,
                text: "Insert your token".to_string()
            }
        );
    }

    #[test]
    fn a_generic_error_is_a_protocol_error() {
        let mut client = GreetdClient::new(ScriptedGreetd::new(vec![Response::Error {
            error_type: ErrorType::Error,
            description: "greetd is busy".to_string(),
        }]));
        let err = client.create_session("alice").unwrap_err();
        assert!(matches!(err, GreetdError::Protocol(_)), "got {err:?}");
    }

    #[test]
    fn the_client_writes_the_expected_requests() {
        let mut client = GreetdClient::new(ScriptedGreetd::new(vec![Response::Success]));
        client.create_session("alice").expect("create session");
        // The serialised request carries the username and the create-session tag.
        let written = String::from_utf8_lossy(&client.stream.written);
        assert!(written.contains("create_session"), "{written}");
        assert!(written.contains("alice"), "{written}");
    }
}
