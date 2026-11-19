//! Exported API that does not come from the protocol manifest.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::wire::{SourceCapabilities, SourceErrorFrame, SourceErrorIssuePathItem};
use crate::worker::{Reply, Worker};
use crate::{Channel, Event, HandshakeError};

/// Why a call failed.
#[derive(Clone, Debug, PartialEq, uniffi::Error)]
// Plain `Error` would become C#'s `Exception`.
#[uniffi(name = "SimlinkError")]
pub enum Error {
    /// A payload could not be used.
    InvalidArgument { message: String },
    /// The event was not declared when the source was created.
    Undeclared { message: String },
    /// The call needs a source that has not started, or one that has.
    WrongState { message: String },
    /// The command was already answered.
    AlreadyAnswered { message: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Error::InvalidArgument { message }
        | Error::Undeclared { message }
        | Error::WrongState { message }
        | Error::AlreadyAnswered { message }) = self;
        f.write_str(message)
    }
}

impl std::error::Error for Error {}

/// Where a source is in finding and joining Simlink.
#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum ConnectionState {
    /// Looking for Simlink, or waiting before the next attempt.
    Searching,
    /// Joined Simlink at `address`.
    Connected { address: String },
    /// Simlink refused the handshake.
    Rejected {
        /// The code as Simlink sends it, such as `HANDSHAKE_SOURCE_ID_IN_USE`.
        code: String,
        /// The refusal with its details, unless it's one this SDK doesn't know, from a newer Simlink.
        error: Option<HandshakeError>,
        message: String,
        /// False when the refusal would repeat, such as an invalid descriptor; the source then stops.
        retrying: bool,
    },
}

/// Connection progress. Called on the SDK's thread.
#[uniffi::export(with_foreign)]
pub trait StateListener: Send + Sync {
    fn changed(&self, state: ConnectionState);
}

/// Simlink found an emitted payload or answer invalid. Called on the SDK's thread.
#[uniffi::export(with_foreign)]
pub trait RejectionListener: Send + Sync {
    fn rejected(&self, rejection: Rejection);
}

/// Why Simlink rejected a payload or answer.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct Rejection {
    pub channel: Channel,
    /// The code as Simlink sends it, such as `INVALID_DATA`.
    pub code: String,
    pub message: Option<String>,
    /// Each field that failed validation.
    pub issues: Vec<RejectionIssue>,
}

/// One field that failed validation.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RejectionIssue {
    /// Where in the payload, such as `aircraft.position.latitude` or `[3].key`.
    pub path: String,
    /// The validator's code, such as `invalid_type`.
    pub code: String,
    pub message: String,
}

type CommandFn = Box<dyn Fn(Value, PendingReply) + Send + Sync>;

/// Where the SDK thread delivers commands, connection changes and rejections.
#[derive(Default)]
pub(crate) struct Listeners {
    commands: HashMap<&'static str, CommandFn>,
    state: Option<Arc<dyn StateListener>>,
    rejected: Option<Arc<dyn RejectionListener>>,
}

impl Listeners {
    pub(crate) fn command(&self, name: &str, input: Value, reply: Reply) {
        match self.commands.get(name) {
            Some(handle) => shield(name, || {
                handle(input, PendingReply(Mutex::new(Some(reply))));
            }),
            None => log::warn!("No handler for {name}"),
        }
    }

    pub(crate) fn state(&self, state: ConnectionState) {
        let Some(listener) = &self.state else { return };
        shield("the state listener", || listener.changed(state));
    }

    pub(crate) fn rejected(&self, frame: SourceErrorFrame) {
        let Some(listener) = &self.rejected else {
            return;
        };
        let rejection = Rejection {
            channel: Channel::read(&frame.event),
            code: frame.code,
            message: frame.message,
            issues: frame
                .issues
                .into_iter()
                .map(|issue| RejectionIssue {
                    path: issue_path(&issue.path),
                    code: issue.code,
                    message: issue.message,
                })
                .collect(),
        };
        shield("the rejection listener", || listener.rejected(rejection));
    }
}

/// Formats an issue's path segments for display: names joined by dots, array indices in brackets, such as
/// `[3].aircraft.position` from `[3, "aircraft", "position"]`.
fn issue_path(segments: &[SourceErrorIssuePathItem]) -> String {
    let mut path = String::new();
    for segment in segments {
        match segment {
            SourceErrorIssuePathItem::Integer(index) => path.push_str(&format!("[{index}]")),
            SourceErrorIssuePathItem::String(name) if path.is_empty() => path.push_str(name),
            SourceErrorIssuePathItem::String(name) => path.push_str(&format!(".{name}")),
        }
    }
    path
}

fn to_json(value: &impl Serialize) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|e| Error::InvalidArgument {
        message: e.to_string(),
    })
}

/// Runs a foreign callback. A C# or C++ exception arrives as a panic, which would end the SDK's thread.
fn shield(what: &str, call: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(call)).is_err() {
        log::error!("{what} failed; the source carries on");
    }
}

/// What `start` connects with, gathered while configuring.
#[derive(Default)]
struct Setup {
    listeners: Listeners,
    address: Option<String>,
}

enum Lifecycle {
    Configuring(Setup),
    Running(Worker),
}

/// A connection to Simlink. Register handlers and listeners, then `start`. Dropping it disconnects.
///
/// Handlers and listeners must not hold their `Source`: the source owns them, so it would never be dropped.
#[derive(uniffi::Object)]
pub struct Source {
    options: crate::SourceOptions,
    events: Vec<Event>,
    lifecycle: Mutex<Lifecycle>,
}

#[uniffi::export]
impl Source {
    /// A source that will declare `events` and the commands it gets handlers for.
    #[uniffi::constructor]
    pub fn new(options: crate::SourceOptions, events: Vec<Event>) -> Source {
        Source {
            options,
            events,
            lifecycle: Mutex::new(Lifecycle::Configuring(Setup::default())),
        }
    }

    /// Register before `start`.
    pub fn on_state(&self, listener: Arc<dyn StateListener>) -> Result<(), Error> {
        self.configure(|setup| setup.listeners.state = Some(listener))
    }

    /// Register before `start`.
    pub fn on_rejected(&self, listener: Arc<dyn RejectionListener>) -> Result<(), Error> {
        self.configure(|setup| setup.listeners.rejected = Some(listener))
    }

    /// Connects with the declared events and the commands handled so far. Keeps reconnecting until dropped.
    pub fn start(&self) -> Result<(), Error> {
        let mut lifecycle = self.lifecycle.lock().expect("source lock");
        let Lifecycle::Configuring(setup) = &mut *lifecycle else {
            return Err(Error::WrongState {
                message: "the source is already started".into(),
            });
        };

        let mut descriptor =
            serde_json::to_value(&self.options).expect("options serialize to JSON");
        let capabilities = SourceCapabilities {
            events: self
                .events
                .iter()
                .map(|event| event.info().name.into())
                .collect(),
            commands: setup
                .listeners
                .commands
                .keys()
                .map(|&name| name.into())
                .collect(),
        };
        descriptor["capabilities"] =
            serde_json::to_value(capabilities).expect("capabilities serialize to JSON");

        let Setup { listeners, address } = std::mem::take(setup);
        *lifecycle = Lifecycle::Running(Worker::start(descriptor, address, listeners));

        Ok(())
    }
}

impl Source {
    /// Test-only: connects to `address` (`host:port`) instead of discovering Simlink.
    #[cfg(feature = "testing")]
    #[doc(hidden)]
    pub fn set_address(&self, address: String) -> Result<(), Error> {
        self.configure(|setup| setup.address = Some(address))
    }

    pub(crate) fn emit_json(&self, event: Event, payload: &impl Serialize) -> Result<(), Error> {
        let payload = to_json(payload)?;
        let info = event.info();
        match &*self.lifecycle.lock().expect("source lock") {
            Lifecycle::Running(_) if !self.events.contains(&event) => Err(Error::Undeclared {
                message: format!("\"{}\" is not declared in the source descriptor", info.name),
            }),
            Lifecycle::Running(worker) => worker.emit(info, &payload),
            Lifecycle::Configuring(_) => Err(Error::WrongState {
                message: "start the source before emitting".into(),
            }),
        }
    }

    pub(crate) fn handle<I: DeserializeOwned>(
        &self,
        channel: &'static str,
        handler: impl Fn(I, PendingReply) + Send + Sync + 'static,
    ) -> Result<(), Error> {
        let handle: CommandFn = Box::new(move |input, reply| match serde_json::from_value(input) {
            Ok(input) => handler(input, reply),
            // Simlink validated the input, so this is an SDK and protocol version mismatch.
            Err(error) => log::error!("Could not read {channel} input: {error}"),
        });
        self.configure(|setup| {
            setup.listeners.commands.insert(channel, handle);
        })
    }

    fn configure(&self, change: impl FnOnce(&mut Setup)) -> Result<(), Error> {
        match &mut *self.lifecycle.lock().expect("source lock") {
            Lifecycle::Configuring(setup) => {
                change(setup);
                Ok(())
            }
            Lifecycle::Running(_) => Err(Error::WrongState {
                message: "register before starting the source".into(),
            }),
        }
    }
}

/// One command's answer, given at most once.
pub(crate) struct PendingReply(Mutex<Option<Reply>>);

impl PendingReply {
    pub(crate) fn ok(&self, value: &impl Serialize) -> Result<(), Error> {
        let value = to_json(value)?;
        self.take()?.ok(value);
        Ok(())
    }

    pub(crate) fn refuse(&self, error: &impl Serialize) -> Result<(), Error> {
        let error = to_json(error)?;
        self.take()?.refuse(error);
        Ok(())
    }

    fn take(&self) -> Result<Reply, Error> {
        self.0
            .lock()
            .expect("reply lock")
            .take()
            .ok_or_else(|| Error::AlreadyAnswered {
                message: "this command was already answered".into(),
            })
    }
}
