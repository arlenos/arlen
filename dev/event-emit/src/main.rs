//! Dev dogfood injector: emit one synthetic event onto the Arlen event bus.
//!
//! The headless image carries no eBPF sensor, so the KG-AI loop cannot be
//! exercised by really opening a file. This tool stands in for the sensor: it
//! connects to the event-bus producer socket and emits a `file.opened` (the only
//! event type promotion turns into a File + Project subgraph), so an in-VM
//! dogfood can drive event -> KG promotion -> the agent's capability-scoped read.
//!
//! IT CONFIRMS DELIVERY RATHER THAN CLAIMING IT. It used to print `emitted` and
//! exit 0 as soon as the bytes reached the socket, which is not the same thing:
//! emitting is fire-and-forget, so a rejected event and a delivered one looked
//! identical from here. On 16 August that cost a full diagnosis - two events
//! reported `emitted`, the graph stayed empty, and the reason was in the BUS log
//! (`dropping invalid event error=missing required field: origin`) because
//! `ARLEN_SESSION_ID` was unset in the scratch stack. The SDK detects exactly
//! that and says so through `tracing::error!`, which a binary with no subscriber
//! discards. So this now subscribes before it emits and waits to see its own
//! event come back around, and says nothing at all about success until it does.
//!
//! IT ALSO STANDS IN FOR THE SHELL. `--menu` emits `app.menu.action_invoked`,
//! which is what the desktop-shell puts on the bus when somebody picks an item
//! from an app's menu. Eleven apps draw a menu whose clicks come back to them
//! over that topic and nothing but a unit test had ever put one on the wire, so
//! there was no way to answer "does clicking it make the app do the thing"
//! without a display and a shell. With this, a headless drive can.
//!
//! IT ALSO LISTENS. `--watch` subscribes to a pattern and prints what arrives,
//! which is the other half of the same problem: eleven apps CONSUME from the bus
//! and twenty-one shell surfaces are meant to PUBLISH onto it, and until now the
//! only way to find out whether an app really put something on the wire was to
//! run a shell and look at a screen. A drive can now assert on the wire itself,
//! which is where the app's half of the contract ends.
//!
//! Usage: `arlen-event-emit <absolute-path> [app-id]`
//!        `arlen-event-emit --menu <app-id> <action>`
//!        `arlen-event-emit --shortcut <app-id> <action>`
//!        `arlen-event-emit --watch <pattern> [seconds]`
//! Sockets: `ARLEN_PRODUCER_SOCKET` / `ARLEN_CONSUMER_SOCKET`, else `/run/arlen/`.
//! Session: `ARLEN_SESSION_ID` must name the session the event belongs to.
//! Exit 0 on a CONFIRMED event, 2 on bad args or no session, 1 on emit failure
//! or on an event that never came back.

use os_sdk::event_consumer::{EventConsumer, UnixEventConsumer};
use os_sdk::proto::{
    AmbientClearedPayload, AmbientSetPayload, BadgeSetPayload, FileOpenedPayload,
    PresenceSetPayload, ShortcutActionInvokedPayload, TimelineRecordPayload,
};
use os_sdk::{EventEmitter, UnixEventEmitter};
use prost::Message;
use std::time::Duration;

/// How long to wait for the bus to hand the event back to us.
///
/// Generous for a loopback socket. A rejected event never arrives at all, so this
/// is the difference between "refused" and "slow", and being slow is not a thing
/// a local Unix socket does.
const CONFIRM_WAIT: Duration = Duration::from_secs(3);

/// How long to let a subscription land before emitting against it.
///
/// Two orders of magnitude over the gap that was actually measured, because being
/// slow here costs a fifth of a second on a dev tool and being early makes it
/// report a delivered event as dropped.
const REGISTRATION_SETTLE: Duration = Duration::from_millis(200);

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(first) = args.next() else {
        usage();
        std::process::exit(2);
    };

    // Two modes, and the file one keeps the bare shape it always had so its
    // existing callers are untouched.
    // Listening is its own thing: it emits nothing, so none of the session and
    // producer machinery below applies to it and it returns from here.
    if first == "--watch" {
        let Some(pattern) = args.next() else {
            usage();
            std::process::exit(2);
        };
        let seconds: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(10);
        watch(pattern, Duration::from_secs(seconds)).await;
        return;
    }

    let mode = if first == "--menu" || first == "--shortcut" {
        let (Some(app_id), Some(action)) = (args.next(), args.next()) else {
            usage();
            std::process::exit(2);
        };
        if first == "--shortcut" {
            Mode::Shortcut { app_id, action }
        } else {
            Mode::Menu { app_id, action }
        }
    } else {
        Mode::FileOpened {
            path: first,
            app_id: args.next().unwrap_or_else(|| "dogfood".to_string()),
        }
    };

    // CHECKED HERE, not left to the SDK's log line. `UnixEventEmitter::new` reads
    // the session id and refuses to invent one when it is missing - correct, and
    // it reports the miss with `tracing::error!`, which goes nowhere in a binary
    // that installs no subscriber. The result was a tool that knew the event
    // would be refused and printed `emitted` anyway. An empty origin is a
    // guaranteed rejection at the bus, so there is nothing to send.
    let session = std::env::var("ARLEN_SESSION_ID").unwrap_or_default();
    if session.is_empty() {
        eprintln!(
            "ARLEN_SESSION_ID is unset, so this event would carry an empty origin \
             and the bus would refuse it (missing required field: origin)."
        );
        eprintln!(
            "An event belongs to a session and the id is not something this tool \
             may invent. Name it:  ARLEN_SESSION_ID=dogfood arlen-event-emit ..."
        );
        std::process::exit(2);
    }

    // The SDK's resolution, same as dogfood's, and for the same reason: the
    // written-out fallback stopped naming anything the day the bus went per-user.
    let producer = os_sdk::runtime::socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock")
        .to_string_lossy()
        .into_owned();
    let consumer = os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock")
        .to_string_lossy()
        .into_owned();

    // Subscribe BEFORE emitting. The bus fans out to whoever is registered at the
    // moment an event arrives, so a consumer that registers afterwards has already
    // missed it - the same ordering the integration suite had to learn.
    let mut inbox = match UnixEventConsumer::new(consumer.clone())
        .subscribe(vec![mode.topic().to_string()])
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            eprintln!("cannot watch for the event on {consumer}: {e}");
            eprintln!("without a subscription this tool cannot tell delivery from silence");
            std::process::exit(1);
        }
    };

    // Let the registration take effect. `subscribe` returns once the three lines are
    // WRITTEN, and the bus reads them on a different task from the one serving the
    // producer - so an emit that follows immediately can be dispatched before the
    // registry has the new consumer in it. Measured on 16 August: publish at
    // .901624, `consumer registered` at .901729, and the event was correctly
    // delivered to graph-writer while this tool saw nothing and called it a drop.
    // The protocol has no registration ack, so a settle is the honest way to wait
    // for something that cannot be observed.
    tokio::time::sleep(REGISTRATION_SETTLE).await;

    let payload = match &mode {
        // flags 0 == a plain read-open (O_RDONLY); promotion only keys off the path.
        Mode::FileOpened { path, app_id } => FileOpenedPayload {
            path: path.clone(),
            app_id: app_id.clone(),
            flags: 0,
        }
        .encode_to_vec(),
        // An empty `window_id` is what the shell sends for a menu click: the menu
        // belongs to the focused app, not to one of its windows, and the plugin's
        // menu relay carries only `{app_id, action}` onward.
        // An empty `window_id` broadcasts to every webview of the named app,
        // which is what both surfaces send: a menu and a launcher entry belong
        // to the app rather than to one of its windows.
        Mode::Menu { app_id, action } | Mode::Shortcut { app_id, action } => {
            ShortcutActionInvokedPayload {
                app_id: app_id.clone(),
                action: action.clone(),
                window_id: String::new(),
            }
            .encode_to_vec()
        }
    };

    let emitter = UnixEventEmitter::new(producer);
    if let Err(e) = emitter.emit(mode.topic(), payload).await {
        eprintln!("emit failed: {e}");
        std::process::exit(1);
    }

    // Match on the content rather than the event id: the SDK mints the id inside
    // `emit`, so the caller never learns it. Another producer emitting the same
    // thing in the same three seconds would satisfy this, which on a dev injector
    // is a trade worth making for a confirmation that is otherwise impossible.
    let confirmed = tokio::time::timeout(CONFIRM_WAIT, async {
        while let Some(event) = inbox.recv().await {
            if event.r#type != mode.topic() {
                continue;
            }
            let mine = match &mode {
                Mode::FileOpened { path, .. } => FileOpenedPayload::decode(&event.payload[..])
                    .is_ok_and(|p| p.path == *path),
                Mode::Menu { app_id, action } | Mode::Shortcut { app_id, action } => {
                    ShortcutActionInvokedPayload::decode(&event.payload[..])
                        .is_ok_and(|p| p.app_id == *app_id && p.action == *action)
                }
            };
            if mine {
                return Some(event.origin);
            }
        }
        None
    })
    .await;

    match confirmed {
        Ok(Some(origin)) => {
            println!("delivered {} origin={origin}", mode.describe());
        }
        Ok(None) => {
            eprintln!("the bus closed the subscription before the event came back");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!(
                "{} was written to the socket but never came back within {}s: \
                 the bus accepted the bytes and dropped the event.",
                mode.describe(),
                CONFIRM_WAIT.as_secs()
            );
            eprintln!(
                "The bus log says which field it refused. Run it with RUST_LOG=debug and \
                 look for `dropping invalid event`."
            );
            std::process::exit(1);
        }
    }
}

/// Which event this run puts on the bus.
enum Mode {
    /// The sensor's event: a file was opened, which promotion turns into a graph.
    FileOpened { path: String, app_id: String },
    /// The shell's event: somebody picked an item from an app's menu.
    Menu { app_id: String, action: String },
    /// The launcher's event: somebody picked one of the app's registered
    /// shortcuts out of the waypointer. Same payload as a menu click and a
    /// different topic, because it is a different surface - and the apps use the
    /// same action strings on both, so this is how a drive checks that.
    Shortcut { app_id: String, action: String },
}

impl Mode {
    fn topic(&self) -> &'static str {
        match self {
            Mode::FileOpened { .. } => "file.opened",
            Mode::Menu { .. } => "app.menu.action_invoked",
            Mode::Shortcut { .. } => "app.shortcut.action_invoked",
        }
    }

    /// What to call this event in a line a person reads.
    fn describe(&self) -> String {
        match self {
            Mode::FileOpened { path, .. } => format!("file.opened path={path}"),
            Mode::Shortcut { app_id, action } => {
                format!("app.shortcut.action_invoked app={app_id} action={action}")
            }
            Mode::Menu { app_id, action } => {
                format!("app.menu.action_invoked app={app_id} action={action}")
            }
        }
    }
}

/// Subscribe to `pattern` and print every event that arrives, until `window`
/// runs out. Exit 0 if at least one did, 1 if none.
///
/// One line per event, `saw <type> <detail>`, so a shell drive can grep for the
/// fact it is about. The DETAIL is decoded per topic and only where a drive has
/// needed it: a badge carries an app id and a count, and reading those off the
/// wire is the difference between "mail published something" and "mail published
/// the number that is actually unread". Everything else reports its length,
/// which still separates delivery from silence.
async fn watch(pattern: String, window: Duration) {
    let consumer = os_sdk::runtime::socket_path("ARLEN_CONSUMER_SOCKET", "event-bus-consumer.sock")
        .to_string_lossy()
        .into_owned();
    let mut inbox = match UnixEventConsumer::new(consumer.clone())
        .subscribe(vec![pattern.clone()])
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            eprintln!("cannot subscribe to {pattern} on {consumer}: {e}");
            std::process::exit(1);
        }
    };
    let mut seen = 0usize;
    let _ = tokio::time::timeout(window, async {
        while let Some(event) = inbox.recv().await {
            seen += 1;
            let detail = match event.r#type.as_str() {
                "app.badge.set" => match BadgeSetPayload::decode(&event.payload[..]) {
                    Ok(p) => format!("app_id={} variant={} count={}", p.app_id, p.variant, p.count),
                    Err(e) => format!("undecodable BadgeSetPayload: {e}"),
                },
                // Presence and timeline are what an app tells the graph about
                // itself, so the interesting part is never that something was
                // published - it is WHAT. A drive that only asserted delivery
                // would pass on a window claiming to edit the wrong file.
                "app.presence.set" => match PresenceSetPayload::decode(&event.payload[..]) {
                    Ok(p) => format!(
                        "app_id={} activity={} subject={} auto_clear={} metadata={:?}",
                        p.app_id, p.activity, p.subject, p.auto_clear, p.metadata
                    ),
                    Err(e) => format!("undecodable PresenceSetPayload: {e}"),
                },
                "app.timeline.record" => match TimelineRecordPayload::decode(&event.payload[..]) {
                    Ok(p) => format!(
                        "app_id={} type={} label={} subject={} metadata={:?}",
                        p.app_id, p.r#type, p.label, p.subject, p.metadata
                    ),
                    Err(e) => format!("undecodable TimelineRecordPayload: {e}"),
                },
                // An ambient effect is the one surface a person sees without
                // looking at any window, so what it says matters more than that
                // it was said: the whole point of the intensity cap and the
                // colour enum is that a wash cannot be arbitrary, and a watcher
                // that printed a byte count could not tell a slow accent pulse
                // from a screen-filling red.
                "app.ambient.set" => match AmbientSetPayload::decode(&event.payload[..]) {
                    Ok(p) => format!(
                        "app_id={} effect={} color={} intensity={} speed={} auto_clear_ms={} reason={}",
                        p.app_id, p.effect, p.color, p.intensity, p.speed, p.auto_clear_ms, p.reason
                    ),
                    Err(e) => format!("undecodable AmbientSetPayload: {e}"),
                },
                "app.ambient.cleared" => match AmbientClearedPayload::decode(&event.payload[..]) {
                    Ok(p) => format!("app_id={}", p.app_id),
                    Err(e) => format!("undecodable AmbientClearedPayload: {e}"),
                },
                _ => format!("{} bytes", event.payload.len()),
            };
            println!("saw {} {}", event.r#type, detail);
        }
    })
    .await;
    if seen == 0 {
        eprintln!("nothing matched {pattern} in {}s", window.as_secs());
        std::process::exit(1);
    }
}

fn usage() {
    eprintln!("usage: arlen-event-emit <absolute-path> [app-id]");
    eprintln!("       arlen-event-emit --menu <app-id> <action>");
    eprintln!("       arlen-event-emit --shortcut <app-id> <action>");
    eprintln!("       arlen-event-emit --watch <pattern> [seconds]");
}
