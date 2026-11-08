//! The background thread that owns the connection.

use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::EventInfo;
use crate::connection::{Command, Handler, Reply, State};
use crate::discovery;
use crate::protocol::{ERROR_CHANNEL, NAMESPACE};
use crate::session::{JoinError, Received, Session};
use crate::wire::{self, Inbound};

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(10);

pub enum Op {
    Emit(&'static EventInfo, Value),
    Reply {
        connection: u64,
        ack: u64,
        result: Value,
    },
    Stop,
}

enum Exit {
    Stop,
    Lost(String),
}

pub struct Worker<H> {
    pub descriptor: Value,
    /// Replaces discovery when set.
    pub address: Option<String>,
    pub handler: H,
    pub ops: Receiver<Op>,
    pub replies: Sender<Op>,
}

impl<H: Handler> Worker<H> {
    pub fn run(mut self) {
        let mut backoff = MIN_BACKOFF;
        // Numbers each joined session, so replies to an earlier one are dropped.
        let mut connection = 0;

        loop {
            self.handler.state(&State::Searching);

            let Some(joined) = self.join() else { return };
            match joined {
                Ok((address, session)) => {
                    connection += 1;
                    log::info!("Connected to Simlink at {address}");
                    self.handler.state(&State::Connected { address });

                    let joined_at = Instant::now();
                    match self.serve(session, connection) {
                        Exit::Stop => return,
                        Exit::Lost(reason) => log::warn!("Lost Simlink: {reason}"),
                    }
                    // A Simlink that drops the source straight after joining keeps backing off.
                    if joined_at.elapsed() >= MAX_BACKOFF {
                        backoff = MIN_BACKOFF;
                    }
                }
                Err(JoinError::Rejected { code, message }) => {
                    log::warn!("Simlink refused the source: {code} {message}");
                    // An invalid descriptor fails the same way on every attempt.
                    let permanent = code == "HANDSHAKE_IDENTIFICATION_INVALID";
                    self.handler.state(&State::Rejected {
                        code,
                        message,
                        retrying: !permanent,
                    });

                    if permanent {
                        self.wait_for_stop(None);
                        return;
                    }
                }
                Err(JoinError::Unreachable(reason)) => log::debug!("Simlink not found: {reason}"),
            }

            if self.wait_for_stop(Some(backoff)) {
                return;
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    /// The first instance that answers, or `None` once the source is dropped.
    fn join(&self) -> Option<Result<(SocketAddr, Session), JoinError>> {
        let stopped = std::cell::Cell::new(false);
        let try_all = |addresses: Vec<SocketAddr>| {
            for address in addresses {
                // Probing every candidate can take seconds, which would hold up dropping the source.
                if self.stop_requested() {
                    stopped.set(true);
                    return None;
                }
                match Session::join(address, &self.descriptor) {
                    Ok(session) => return Some(Ok((address, session))),
                    Err(rejected @ JoinError::Rejected { .. }) => return Some(Err(rejected)),
                    Err(JoinError::Unreachable(reason)) => log::trace!("{reason}"),
                }
            }
            None
        };

        let found = match &self.address {
            Some(address) => match address.to_socket_addrs() {
                Ok(resolved) => try_all(resolved.collect()),
                Err(error) => {
                    return Some(Err(JoinError::Unreachable(format!("{address}: {error}"))));
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
            Err(JoinError::Unreachable(
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

    fn serve(&mut self, mut session: Session, connection: u64) -> Exit {
        // Latest payload per event, sent no faster than the event's rate limit.
        let mut pending: HashMap<&'static str, (&'static EventInfo, Value)> = HashMap::new();
        let mut last_sent: HashMap<&'static str, Instant> = HashMap::new();

        loop {
            loop {
                match self.ops.try_recv() {
                    Ok(Op::Emit(event, payload)) => {
                        pending.insert(event.name, (event, payload));
                    }
                    Ok(Op::Reply {
                        connection: target,
                        ack,
                        result,
                    }) if target == connection => {
                        if let Err(reason) = session.send(&wire::ack(NAMESPACE, ack, &result)) {
                            return Exit::Lost(reason);
                        }
                    }
                    Ok(Op::Reply { .. }) => {}
                    Ok(Op::Stop) | Err(TryRecvError::Disconnected) => {
                        session.close();
                        return Exit::Stop;
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            let now = Instant::now();
            let mut wait = POLL_INTERVAL;

            pending.retain(|name, (event, payload)| {
                // Held back, the latest payload replaces older ones instead of queueing behind them.
                if session.backlogged() {
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
                let _ = session.send(&wire::event(NAMESPACE, name, payload));
                last_sent.insert(name, now);
                false
            });

            match session.poll(wait) {
                Received::Frame(inbound) => {
                    if let Some(exit) = self.dispatch(inbound, connection) {
                        return exit;
                    }
                }
                Received::Idle => {}
                Received::Lost(reason) => return Exit::Lost(reason),
            }
        }
    }

    fn dispatch(&mut self, inbound: Inbound, connection: u64) -> Option<Exit> {
        match inbound {
            Inbound::Event {
                name,
                mut args,
                ack,
            } => {
                let payload = if args.is_empty() {
                    Value::Null
                } else {
                    args.swap_remove(0)
                };

                if name == ERROR_CHANNEL {
                    log::warn!("Simlink rejected data: {payload}");
                    self.handler.rejected(&payload);
                } else if let Some(ack) = ack {
                    let reply = Reply {
                        ops: self.replies.clone(),
                        connection,
                        ack,
                    };
                    self.handler.command(Command {
                        name,
                        input: payload,
                        reply,
                    });
                } else {
                    log::debug!("Ignoring unexpected event {name}");
                }

                None
            }
            Inbound::Disconnected => Some(Exit::Lost("Simlink disconnected the source".into())),
            _ => None,
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
