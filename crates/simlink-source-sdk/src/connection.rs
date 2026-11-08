//! The JSON-level connection: one descriptor, channel names and JSON payloads.

use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use serde_json::{Value, json};

use crate::protocol::{MAX_PAYLOAD_BYTES, NAMESPACE};
use crate::wire;
use crate::worker::{Op, Worker};
use crate::{ConnectionState, Error};

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

/// A running connection to Simlink. Dropping it disconnects.
pub struct Connection {
    ops: Sender<Op>,
    worker: Option<JoinHandle<()>>,
}

impl Connection {
    /// Joins with `descriptor`, a `SourceDescriptor` in Simlink's handshake format, at `address` or wherever
    /// discovery finds Simlink.
    pub fn start(descriptor: Value, address: Option<String>, handler: impl Handler) -> Connection {
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

        Connection {
            ops,
            worker: Some(worker),
        }
    }

    /// Queues the latest payload for `event`. Sent at most at the event's rate limit; dropped while disconnected.
    pub fn emit(&self, event: &'static EventInfo, payload: &Value) -> Result<(), Error> {
        let frame = wire::event(NAMESPACE, event.name, payload);
        // Refused here because Simlink would drop the connection instead.
        if frame.len() > MAX_PAYLOAD_BYTES {
            return Err(Error::InvalidArgument {
                message: format!(
                    "\"{}\" is {} bytes; Simlink accepts messages up to {MAX_PAYLOAD_BYTES} bytes",
                    event.name,
                    frame.len()
                ),
            });
        }

        // The worker only exits after drop.
        let _ = self.ops.send(Op::Emit(event, frame));

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
