//! The SDK's thread: finds Simlink, joins with the source's descriptor, sends each event's latest payload at its
//! rate limit, and hands commands and rejections to the listeners. The Socket.IO client reconnects.

use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use socket_io::{
    Addresses, Backoff, Client, ConnectError, DisconnectReason, Event, Frame, Options, Responder,
    Waker,
};

use crate::discovery;
use crate::protocol::{ERROR_CHANNEL, MAX_PAYLOAD_BYTES, NAMESPACE, SOCKET_IO_PATH};
use crate::source::Listeners;
use crate::{ConnectionState, Error, HandshakeError};

const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);
// Short, since every candidate address is tried in turn.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// An event channel a source can emit.
#[derive(Debug)]
pub struct EventInfo {
    pub name: &'static str,
    /// Simlink coalesces emissions above this rate, keeping the latest.
    pub max_rate_hz: Option<f64>,
}

enum Op {
    Emit(&'static EventInfo, Frame),
    Reply { responder: Responder, result: Value },
    Stop,
}

/// Sends to the SDK thread and wakes it.
#[derive(Clone)]
struct Ops {
    sender: Sender<Op>,
    waker: Waker,
}

impl Ops {
    fn send(&self, op: Op) {
        // Fails only once the thread has exited, when nothing is listening.
        let _ = self.sender.send(op);
        self.waker.wake();
    }
}

/// Answers one command. Dropping it unanswered lets the command time out.
pub struct Reply {
    ops: Ops,
    responder: Responder,
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
        self.ops.send(Op::Reply {
            responder: self.responder,
            result,
        });
    }
}

/// The running SDK thread. Dropping it disconnects.
pub struct Worker {
    ops: Ops,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    /// Joins with `descriptor`, a `SourceDescriptor` in Simlink's handshake format, at `address` or wherever
    /// discovery finds Simlink.
    pub fn start(descriptor: Value, address: Option<String>, listeners: Listeners) -> Worker {
        let client = Client::new(
            move || candidates(address.as_deref()),
            NAMESPACE,
            descriptor.as_object().cloned(),
            Options {
                path: SOCKET_IO_PATH.into(),
                reconnection_delay: MIN_BACKOFF,
                reconnection_delay_max: MAX_BACKOFF,
                timeout: HANDSHAKE_TIMEOUT,
                ..Options::default()
            },
        )
        .expect("the OS refused the SDK a poller");
        let (sender, receiver) = mpsc::channel();
        let ops = Ops {
            sender,
            waker: client.waker(),
        };
        let thread = Thread {
            listeners,
            ops: receiver,
            replies: ops.clone(),
        };
        let thread = std::thread::Builder::new()
            .name("simlink-source".into())
            .spawn(move || thread.run(client))
            .expect("the OS refused to start the SDK thread");

        Worker {
            ops,
            thread: Some(thread),
        }
    }

    /// Queues the latest payload for `event`. Sent at most at the event's rate limit; dropped while disconnected.
    pub fn emit(&self, event: &'static EventInfo, payload: &Value) -> Result<(), Error> {
        let frame =
            Frame::event(NAMESPACE, event.name, vec![payload.clone()]).map_err(|error| {
                Error::InvalidArgument {
                    message: error.to_string(),
                }
            })?;
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

        self.ops.send(Op::Emit(event, frame));

        Ok(())
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.ops.send(Op::Stop);

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

/// What the SDK thread owns.
struct Thread {
    listeners: Listeners,
    ops: Receiver<Op>,
    replies: Ops,
}

impl Thread {
    fn run(self, mut client: Client) {
        self.listeners.state(ConnectionState::Searching);

        // Latest frame per event, sent no faster than the event's rate limit.
        let mut pending: HashMap<&'static str, (&'static EventInfo, Frame)> = HashMap::new();
        let mut last_sent: HashMap<&'static str, Instant> = HashMap::new();
        // When to rejoin after Simlink ends the source.
        let mut rejoin_at: Option<Instant> = None;
        let mut rejoin_backoff = Backoff::new(MIN_BACKOFF, MAX_BACKOFF);
        let mut connected_at = None;

        loop {
            loop {
                match self.ops.try_recv() {
                    Ok(Op::Emit(event, frame)) => {
                        pending.insert(event.name, (event, frame));
                    }
                    Ok(Op::Reply { responder, result }) => {
                        // Ack ids start over with each connection, so a stale reply could answer another command,
                        // and Simlink closes a connection that acks before joining.
                        if client.connected() && client.is_current(&responder) {
                            client.ack(responder, vec![result]);
                        } else {
                            log::debug!("Dropping a reply to a command from an earlier connection");
                        }
                    }
                    Ok(Op::Stop) | Err(TryRecvError::Disconnected) => {
                        client.close();
                        return;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            let now = Instant::now();
            // Until the next due emit or rejoin; ops, socket activity and a drained backlog wake it sooner.
            let mut wait = Duration::MAX;

            if !client.connected() {
                pending.clear();
            } else if client.writable() {
                let mut due = Vec::new();
                for (name, (event, _)) in &pending {
                    let due_at = match (event.max_rate_hz, last_sent.get(name)) {
                        (Some(rate), Some(sent)) => *sent + Duration::from_secs_f64(1.0 / rate),
                        _ => now,
                    };
                    if due_at > now {
                        wait = wait.min(due_at - now);
                    } else {
                        due.push(*name);
                    }
                }
                for name in due {
                    // Sending can fill the socket, so later events wait for the next pass.
                    if !client.writable() {
                        break;
                    }
                    if let Some((_, frame)) = pending.remove(name) {
                        client.emit(frame);
                        last_sent.insert(name, now);
                    }
                }
            }

            if let Some(at) = rejoin_at {
                if at <= now {
                    rejoin_at = None;
                    self.listeners.state(ConnectionState::Searching);
                    client.join();
                } else {
                    wait = wait.min(at - now);
                }
            }

            let Some(event) = client.poll(wait) else {
                continue;
            };
            match event {
                Event::Connect { address } => {
                    log::info!("Connected to Simlink at {address}");
                    connected_at = Some(Instant::now());
                    self.listeners.state(ConnectionState::Connected {
                        address: address.to_string(),
                    });
                }
                Event::Disconnect(reason) => {
                    // Also reported for a connection that closed before Simlink accepted the source.
                    match connected_at.take() {
                        Some(at) => {
                            log::warn!("Lost Simlink: {reason}");
                            self.listeners.state(ConnectionState::Searching);
                            // A Simlink that drops the source straight after joining keeps backing off.
                            if at.elapsed() >= MAX_BACKOFF {
                                rejoin_backoff.reset();
                            }
                        }
                        None => log::debug!(
                            "Simlink closed the connection before accepting the source: {reason}"
                        ),
                    }
                    // The client rejoins after every other reason.
                    if reason == DisconnectReason::ServerDisconnect {
                        rejoin_at = Some(Instant::now() + rejoin_backoff.duration());
                    }
                }
                Event::ConnectError(ConnectError::Refused { message, data }) => {
                    let data = data.unwrap_or_default();
                    let error = HandshakeError::read(
                        data["code"].as_str().unwrap_or_default(),
                        &data["metadata"],
                    );
                    log::warn!(
                        "Simlink refused the source: {} {message}",
                        error.wire_code()
                    );
                    // An invalid descriptor fails the same way on every attempt.
                    let permanent = matches!(error, HandshakeError::IdentificationInvalid { .. });
                    self.listeners.state(ConnectionState::Rejected {
                        error,
                        message,
                        retrying: !permanent,
                    });

                    if permanent {
                        client.close();
                        self.wait_for_stop();
                        return;
                    }
                    rejoin_at = Some(Instant::now() + rejoin_backoff.duration());
                }
                Event::ConnectError(ConnectError::Transport(reason)) => {
                    log::debug!("Simlink not found: {reason}")
                }
                // A Socket.IO v2 server answers this way; it sends no pings, so the heartbeat drops it.
                Event::ConnectError(ConnectError::UnsupportedServer) => {
                    log::warn!("Not a Simlink: the server answered without a session id")
                }
                Event::Message { name, args, ack } => self.dispatch(name, args, ack),
            }
        }
    }

    fn dispatch(&self, name: String, args: Vec<Value>, ack: Option<Responder>) {
        let payload = args.into_iter().next().unwrap_or(Value::Null);

        if name == ERROR_CHANNEL {
            log::warn!("Simlink rejected data: {payload}");
            self.listeners.rejected(&payload);
        } else if let Some(responder) = ack {
            let reply = Reply {
                ops: self.replies.clone(),
                responder,
            };
            self.listeners.command(&name, payload, reply);
        } else {
            log::debug!("Ignoring unexpected event {name}");
        }
    }

    /// Blocks until the source is dropped. Emits and replies are dropped meanwhile.
    fn wait_for_stop(&self) {
        while let Ok(op) = self.ops.recv() {
            if let Op::Stop = op {
                return;
            }
        }
    }
}

/// Where to look for Simlink on each attempt: `address` when set, otherwise this machine's ports, then mDNS.
fn candidates(address: Option<&str>) -> Addresses {
    match address {
        Some(address) => match address.to_socket_addrs() {
            Ok(resolved) => Box::new(resolved),
            Err(error) => {
                log::debug!("{address}: {error}");
                Box::new(std::iter::empty())
            }
        },
        // Lazy, so mDNS is only browsed once no local port answers.
        None => Box::new(
            std::iter::once_with(discovery::loopback_candidates)
                .flatten()
                .chain(std::iter::once_with(discovery::browse_mdns).flatten()),
        ),
    }
}
