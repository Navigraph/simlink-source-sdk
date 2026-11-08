//! The JSON-level connection: one descriptor, channel names and JSON payloads.

use std::collections::HashSet;
use std::fmt;
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use serde_json::{Value, json};

use crate::ConnectionState;
use crate::protocol::{COMMANDS, EVENTS, MAX_PAYLOAD_BYTES, NAMESPACE};
use crate::wire;
use crate::worker::{Op, Worker};

/// An event channel a source can emit.
#[derive(Debug)]
pub struct EventInfo {
    pub name: &'static str,
    /// Simlink coalesces emissions above this rate, keeping the latest.
    pub max_rate_hz: Option<f64>,
}

/// Receives commands and connection updates on the SDK's thread.
pub trait Handler: Send + 'static {
    /// Answer through [`Command::reply`], now or later from any thread.
    fn command(&mut self, command: Command);

    fn state(&mut self, _state: ConnectionState) {}

    /// Simlink found an emitted payload or reply invalid. `frame` is its error frame.
    fn rejected(&mut self, _frame: &Value) {}
}

/// A command from Simlink awaiting one answer.
#[derive(Debug)]
pub struct Command {
    pub name: String,
    pub input: Value,
    pub reply: Reply,
}

/// Answers one command. Dropping it unanswered lets the command time out.
#[derive(Debug)]
pub struct Reply {
    pub(crate) ops: Sender<Op>,
    pub(crate) connection: u64,
    pub(crate) ack: u64,
}

impl Reply {
    pub fn ok(self, value: Value) {
        self.send(json!({ "ok": true, "value": value }));
    }

    /// Refuses with one of the command's declared error codes.
    pub fn refuse(self, error: Value) {
        self.send(json!({ "ok": false, "error": error }));
    }

    fn send(self, result: Value) {
        // The source was dropped; nothing is listening.
        let _ = self.ops.send(Op::Reply {
            connection: self.connection,
            ack: self.ack,
            result,
        });
    }
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// The descriptor's capabilities are malformed. Simlink validates the rest.
    InvalidDescriptor(String),
    /// The channel is not part of the protocol this SDK was built from.
    UnknownChannel(String),
    /// The channel was not declared in the descriptor.
    Undeclared(String),
    /// The payload's message is over Simlink's limit, in bytes.
    TooLarge(String, usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidDescriptor(reason) => write!(f, "invalid descriptor: {reason}"),
            Error::UnknownChannel(name) => {
                write!(f, "\"{name}\" is not a Simlink protocol channel")
            }
            Error::Undeclared(name) => {
                write!(f, "\"{name}\" is not declared in the source descriptor")
            }
            Error::TooLarge(name, size) => write!(
                f,
                "\"{name}\" is {size} bytes; Simlink accepts messages up to {MAX_PAYLOAD_BYTES} bytes"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// A running connection to Simlink. Dropping it disconnects.
pub struct Connection {
    ops: Sender<Op>,
    events: HashSet<&'static str>,
    worker: Option<JoinHandle<()>>,
}

impl Connection {
    /// Joins with `descriptor`, a `SourceDescriptor` in Simlink's handshake format, at `address` or wherever
    /// discovery finds Simlink.
    pub fn start(
        descriptor: Value,
        address: Option<String>,
        handler: impl Handler,
    ) -> Result<Connection, Error> {
        let events = declared(&descriptor, "events")?
            .into_iter()
            .map(|name| {
                event_info(name)
                    .map(|event| event.name)
                    .ok_or_else(|| Error::UnknownChannel(name.into()))
            })
            .collect::<Result<_, _>>()?;

        if let Some(unknown) = declared(&descriptor, "commands")?
            .into_iter()
            .find(|name| !COMMANDS.contains(name))
        {
            return Err(Error::UnknownChannel(unknown.into()));
        }

        let (ops, receiver) = mpsc::channel();
        let worker = Worker {
            descriptor,
            address,
            handler,
            ops: receiver,
            replies: ops.clone(),
        };
        let worker = std::thread::Builder::new()
            .name("simlink-source".into())
            .spawn(move || worker.run())
            .expect("the OS refused to start the SDK thread");

        Ok(Connection {
            ops,
            events,
            worker: Some(worker),
        })
    }

    /// Queues the latest payload for `event`. Sent at most at the event's rate limit; dropped while disconnected.
    pub fn emit(&self, event: &str, payload: Value) -> Result<(), Error> {
        let info = event_info(event).ok_or_else(|| Error::UnknownChannel(event.into()))?;
        if !self.events.contains(info.name) {
            return Err(Error::Undeclared(event.into()));
        }
        // Refused here because Simlink would drop the connection instead.
        let size = wire::event(NAMESPACE, info.name, &payload).len();
        if size > MAX_PAYLOAD_BYTES {
            return Err(Error::TooLarge(event.into(), size));
        }

        // The worker only exits after drop.
        let _ = self.ops.send(Op::Emit(info, payload));

        Ok(())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.ops.send(Op::Stop);

        // Dropped from a handler callback: the worker exits once it returns.
        if let Some(worker) = self
            .worker
            .take()
            .filter(|worker| worker.thread().id() != std::thread::current().id())
        {
            let _ = worker.join();
        }
    }
}

fn event_info(name: &str) -> Option<&'static EventInfo> {
    EVENTS.iter().find(|event| event.name == name)
}

/// The channel names under `capabilities.<key>`.
fn declared<'a>(descriptor: &'a Value, key: &str) -> Result<Vec<&'a str>, Error> {
    descriptor["capabilities"][key]
        .as_array()
        .and_then(|names| names.iter().map(Value::as_str).collect())
        .ok_or_else(|| {
            Error::InvalidDescriptor(format!("capabilities.{key} must be an array of strings"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ignore;

    impl Handler for Ignore {
        fn command(&mut self, _: Command) {}
    }

    fn descriptor(events: Value) -> Value {
        json!({
            "id": "test-source",
            "displayName": "Test",
            "capabilities": { "events": events, "commands": ["atis.get"] },
        })
    }

    #[test]
    fn rejects_malformed_capabilities_before_connecting() {
        assert_eq!(
            declared(&descriptor(json!(["telemetry.ownship"])), "events"),
            Ok(vec!["telemetry.ownship"])
        );
        assert!(matches!(
            declared(&descriptor(json!("telemetry.ownship")), "events"),
            Err(Error::InvalidDescriptor(_))
        ));
        assert!(matches!(
            declared(&json!({}), "commands"),
            Err(Error::InvalidDescriptor(_))
        ));
    }

    #[test]
    fn rejects_unknown_channels_before_connecting() {
        assert_eq!(
            Connection::start(descriptor(json!(["telemetry.nope"])), None, Ignore).err(),
            Some(Error::UnknownChannel("telemetry.nope".into()))
        );
    }
}
