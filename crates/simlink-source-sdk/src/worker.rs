//! The SDK's thread: finds Simlink, joins with the source's descriptor, sends each event's latest payload at its
//! rate limit, hands commands and rejections to the listeners, and reconnects with backoff.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::discovery;
use crate::protocol::{ERROR_CHANNEL, MAX_PAYLOAD_BYTES, NAMESPACE, SOCKET_IO_PATH};
use crate::socket_io::{ConnectError, Frame, Received, Socket};
use crate::source::Listeners;
use crate::{ConnectionState, Error};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// An event channel a source can emit.
#[derive(Debug)]
pub struct EventInfo {
    pub name: &'static str,
    /// Simlink coalesces emissions above this rate, keeping the latest.
    pub max_rate_hz: Option<f64>,
}

enum Op {
    Emit(&'static EventInfo, Frame),
    Reply {
        connection: u64,
        ack: u64,
        result: Value,
    },
    Stop,
}

/// Answers one command. Dropping it unanswered lets the command time out.
pub struct Reply {
    ops: Sender<Op>,
    connection: u64,
    ack: u64,
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

/// The running SDK thread. Dropping it disconnects.
pub struct Worker {
    ops: Sender<Op>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Joins with `descriptor`, a `SourceDescriptor` in Simlink's handshake format, at `address` or wherever
    /// discovery finds Simlink.
    pub fn start(descriptor: Value, address: Option<String>, listeners: Listeners) -> Worker {
        let (ops, receiver) = mpsc::channel();
        let thread = Thread {
            descriptor,
            address,
            listeners,
            ops: receiver,
            replies: ops.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("simlink-source".into())
            .spawn(move || thread.run())
            .expect("the OS refused to start the SDK thread");

        Worker {
            ops,
            thread: Some(thread),
        }
    }

    /// Queues the latest payload for `event`. Sent at most at the event's rate limit; dropped while disconnected.
    pub fn emit(&self, event: &'static EventInfo, payload: &Value) -> Result<(), Error> {
        let frame = Frame::event(NAMESPACE, event.name, payload);
        // Refused here because Simlink would drop the connection instead.
        if frame.size() > MAX_PAYLOAD_BYTES {
            return Err(Error::InvalidArgument {
                message: format!(
                    "\"{}\" is {} bytes; Simlink accepts messages up to {MAX_PAYLOAD_BYTES} bytes",
                    event.name,
                    frame.size()
                ),
            });
        }

        // The thread only exits after drop.
        let _ = self.ops.send(Op::Emit(event, frame));

        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.ops.send(Op::Stop);

        // Dropped from a listener callback: the thread exits once it returns.
        if let Some(thread) = self
            .thread
            .take()
            .filter(|thread| thread.thread().id() != std::thread::current().id())
        {
            let _ = thread.join();
        }
    }
}

enum Exit {
    Stop,
    Lost(String),
}

/// What the SDK thread owns.
struct Thread {
    descriptor: Value,
    /// Replaces discovery when set.
    address: Option<String>,
    listeners: Listeners,
    ops: Receiver<Op>,
    replies: Sender<Op>,
}

impl Thread {
    fn run(self) {
        let mut backoff = MIN_BACKOFF;
        // Numbers each joined socket, so replies to an earlier one are dropped.
        let mut connection = 0;

        loop {
            self.listeners.state(ConnectionState::Searching);

            let Some(joined) = self.join() else { return };
            match joined {
                Ok((address, socket)) => {
                    connection += 1;
                    log::info!("Connected to Simlink at {address}");
                    self.listeners.state(ConnectionState::Connected {
                        address: address.to_string(),
                    });

                    let joined_at = Instant::now();
                    match self.serve(socket, connection) {
                        Exit::Stop => return,
                        Exit::Lost(reason) => log::warn!("Lost Simlink: {reason}"),
                    }
                    // A Simlink that drops the source straight after joining keeps backing off.
                    if joined_at.elapsed() >= MAX_BACKOFF {
                        backoff = MIN_BACKOFF;
                    }
                }
                Err(ConnectError::Refused { message, data }) => {
                    let code = data
                        .as_ref()
                        .and_then(|data| data.get("code"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    log::warn!("Simlink refused the source: {code} {message}");
                    // An invalid descriptor fails the same way on every attempt.
                    let permanent = code == "HANDSHAKE_IDENTIFICATION_INVALID";
                    self.listeners.state(ConnectionState::Rejected {
                        code,
                        message,
                        retrying: !permanent,
                    });

                    if permanent {
                        self.wait_for_stop(None);
                        return;
                    }
                }
                Err(ConnectError::Unreachable(reason)) => {
                    log::debug!("Simlink not found: {reason}")
                }
            }

            if self.wait_for_stop(Some(backoff)) {
                return;
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    /// The first instance that answers, or `None` once the source is dropped.
    fn join(&self) -> Option<Result<(SocketAddr, Socket), ConnectError>> {
        let stopped = std::cell::Cell::new(false);
        let try_all = |addresses: Vec<SocketAddr>| {
            for address in addresses {
                // Probing every candidate can take seconds, which would hold up dropping the source.
                if self.stop_requested() {
                    stopped.set(true);
                    return None;
                }
                match Socket::connect(address, SOCKET_IO_PATH, NAMESPACE, &self.descriptor) {
                    Ok(socket) => return Some(Ok((address, socket))),
                    Err(refused @ ConnectError::Refused { .. }) => return Some(Err(refused)),
                    Err(ConnectError::Unreachable(reason)) => log::trace!("{reason}"),
                }
            }
            None
        };

        let found = match &self.address {
            Some(address) => match address.to_socket_addrs() {
                Ok(resolved) => try_all(resolved.collect()),
                Err(error) => {
                    return Some(Err(ConnectError::Unreachable(format!(
                        "{address}: {error}"
                    ))));
                }
            },
            // mDNS is only browsed when no instance answers on this machine.
            None => try_all(discovery::loopback_candidates()).or_else(|| {
                if stopped.get() {
                    None
                } else {
                    try_all(discovery::browse_mdns())
                }
            }),
        };

        if stopped.get() {
            return None;
        }
        Some(found.unwrap_or_else(|| {
            Err(ConnectError::Unreachable(
                "no Simlink instance answered".into(),
            ))
        }))
    }

    /// Whether the source was dropped. Emits and replies queued meanwhile are dropped, as while disconnected.
    fn stop_requested(&self) -> bool {
        loop {
            match self.ops.try_recv() {
                Ok(Op::Stop) | Err(TryRecvError::Disconnected) => return true,
                Ok(_) => {}
                Err(TryRecvError::Empty) => return false,
            }
        }
    }

    fn serve(&self, mut socket: Socket, connection: u64) -> Exit {
        // Latest frame per event, sent no faster than the event's rate limit.
        let mut pending: HashMap<&'static str, (&'static EventInfo, Frame)> = HashMap::new();
        let mut last_sent: HashMap<&'static str, Instant> = HashMap::new();

        loop {
            loop {
                match self.ops.try_recv() {
                    Ok(Op::Emit(event, frame)) => {
                        pending.insert(event.name, (event, frame));
                    }
                    Ok(Op::Reply {
                        connection: target,
                        ack,
                        result,
                    }) if target == connection => {
                        if let Err(reason) = socket.ack(ack, &result) {
                            return Exit::Lost(reason);
                        }
                    }
                    Ok(Op::Reply { .. }) => {}
                    Ok(Op::Stop) | Err(TryRecvError::Disconnected) => {
                        socket.close();
                        return Exit::Stop;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            let now = Instant::now();
            let mut wait = POLL_INTERVAL;

            pending.retain(|name, (event, frame)| {
                // Held back, the latest payload replaces older ones instead of queueing behind them.
                if socket.backlogged() {
                    return true;
                }
                let due = match (event.max_rate_hz, last_sent.get(name)) {
                    (Some(rate), Some(sent)) => *sent + Duration::from_secs_f64(1.0 / rate),
                    _ => now,
                };

                if due > now {
                    wait = wait.min(due - now);
                    return true;
                }

                // A failed send surfaces as a lost connection on the next poll.
                let _ = socket.send(frame);
                last_sent.insert(name, now);
                false
            });

            match socket.poll(wait) {
                Received::Event { name, args, ack } => self.dispatch(name, args, ack, connection),
                Received::Idle => {}
                Received::Lost(reason) => return Exit::Lost(reason),
            }
        }
    }

    fn dispatch(&self, name: String, mut args: Vec<Value>, ack: Option<u64>, connection: u64) {
        let payload = if args.is_empty() {
            Value::Null
        } else {
            args.swap_remove(0)
        };

        if name == ERROR_CHANNEL {
            log::warn!("Simlink rejected data: {payload}");
            self.listeners.rejected(&payload);
        } else if let Some(ack) = ack {
            let reply = Reply {
                ops: self.replies.clone(),
                connection,
                ack,
            };
            self.listeners.command(&name, payload, reply);
        } else {
            log::debug!("Ignoring unexpected event {name}");
        }
    }

    /// Sleeps for `duration`, or until stopped when `None`. Emits and replies are dropped meanwhile.
    fn wait_for_stop(&self, duration: Option<Duration>) -> bool {
        let deadline = duration.map(|duration| Instant::now() + duration);

        loop {
            let received = match deadline {
                Some(deadline) => self
                    .ops
                    .recv_timeout(deadline.saturating_duration_since(Instant::now())),
                None => self.ops.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };

            match received {
                Ok(Op::Stop) | Err(RecvTimeoutError::Disconnected) => return true,
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => return false,
            }
        }
    }
}
