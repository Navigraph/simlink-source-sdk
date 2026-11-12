//! The connection and its namespace, after the reference client's `Manager` and `Socket`.
//! <https://github.com/socketio/socket.io/tree/main/packages/socket.io-client/lib>

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::backoff::Backoff;
use crate::engine::{Connection, Read};
use crate::waker::{Signal, Waker};
use crate::wire::{self, Inbound};
use crate::{ConnectError, DisconnectReason, Event, Frame, Options, Responder};

/// Where to look on each attempt, pulled lazily, only as far as needed.
pub type Addresses = Box<dyn Iterator<Item = SocketAddr> + Send>;

/// A Socket.IO client joined to one namespace.
pub struct Client {
    addresses: Box<dyn FnMut() -> Addresses + Send>,
    options: Options,
    namespace: String,
    auth: Option<Map<String, Value>>,
    state: State,
    /// Counts connections; see [`Client::is_current`].
    connection: u64,
    backoff: Backoff,
    /// Cleared when the server ends the namespace.
    joining: bool,
    connected: bool,
    /// Events that arrived before the `CONNECT` answer. Like the reference `receiveBuffer`, it survives a lost
    /// connection.
    received_early: Vec<Event>,
    /// Frames emitted before the `CONNECT` answer. Like the reference `sendBuffer`, it survives a lost connection.
    sent_early: Vec<String>,
    /// Whether the last connection was writable when it closed. The reference emit keeps checking the closed
    /// transport until the next attempt replaces it.
    was_writable: bool,
    events: VecDeque<Event>,
    signal: Arc<Signal>,
}

enum State {
    Idle,
    /// Finding the next address and opening it are separate steps, since either can block.
    Opening {
        candidates: Option<Addresses>,
        next: Option<SocketAddr>,
        error: Option<String>,
        /// Acks, sent once a connection opens. Reference: engine.io-client's `writeBuffer`.
        acks: Vec<String>,
    },
    Open(Box<Connection>),
    Waiting {
        until: Instant,
    },
}

impl Client {
    /// Joins `namespace` (`/` or e.g. `/admin`) with `auth` at the first of `addresses` that opens a session.
    /// Connects on the first [`Client::poll`]. Fails if the OS won't create a poller.
    pub fn new(
        addresses: impl FnMut() -> Addresses + Send + 'static,
        namespace: &str,
        auth: Option<Map<String, Value>>,
        options: Options,
    ) -> std::io::Result<Client> {
        let backoff = Backoff::new(options.reconnection_delay, options.reconnection_delay_max);
        Ok(Client {
            addresses: Box::new(addresses),
            options,
            namespace: namespace.to_owned(),
            auth,
            state: State::Opening {
                candidates: None,
                next: None,
                error: None,
                acks: Vec::new(),
            },
            connection: 0,
            backoff,
            joining: true,
            connected: false,
            received_early: Vec::new(),
            sent_early: Vec::new(),
            was_writable: false,
            events: VecDeque::new(),
            signal: Signal::new()?,
        })
    }

    /// Wakes [`Client::poll`] from another thread.
    pub fn waker(&self) -> Waker {
        Waker(self.signal.clone())
    }

    /// Rejoins after the server refused or disconnected the namespace, or after a lost connection without
    /// [`Options::reconnection`].
    pub fn join(&mut self) {
        if matches!(self.state, State::Idle) {
            self.joining = true;
            self.open();
        }
    }

    /// Sends like a reference volatile event: dropped while a connection is opening or backlogged, held until joined
    /// (also after a writable connection closes, as the reference checks its last transport), sent otherwise.
    pub fn emit(&mut self, frame: Frame) {
        let (writable, ping_expired) = match &self.state {
            State::Open(connection) => (connection.writable(), connection.ping_expired()),
            _ => (self.was_writable, false),
        };
        if !writable {
            return;
        }
        if self.connected && !ping_expired {
            self.send(frame.0);
        } else {
            self.sent_early.push(frame.0);
            // Reference: `_hasPingExpired` closes the connection as soon as it notices.
            if self.connected {
                self.on_close(DisconnectReason::PingTimeout);
            }
        }
    }

    /// Answers a server event over the current connection, even if the event came on an earlier one; see
    /// [`Client::is_current`]. While a connection opens, it's queued ahead of `CONNECT`, which the reference server
    /// answers by closing the connection. With none open or opening, it's dropped.
    ///
    /// <https://github.com/socketio/socket.io-protocol#acknowledgement>
    pub fn ack(&mut self, responder: Responder, args: Vec<Value>) {
        let ack = wire::ack(&self.namespace, responder.id, args);
        match &mut self.state {
            State::Opening { acks, .. } => acks.push(ack),
            _ => self.send(ack),
        }
    }

    /// Whether the event came over the current connection. Ack ids restart per connection, so a stale answer could
    /// reach a different event.
    pub fn is_current(&self, responder: &Responder) -> bool {
        responder.connection == self.connection
    }

    /// Whether the namespace is joined.
    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Whether a frame sent now goes straight out instead of queueing behind ones the server hasn't read.
    pub fn writable(&self) -> bool {
        matches!(&self.state, State::Open(connection) if connection.writable())
    }

    /// Runs the client for up to `wait` and returns the next event, sleeping while there's nothing to do. Returns
    /// `None` early when a [`Waker`] fires or a backlogged connection becomes [`writable`](Client::writable) again.
    /// Can overrun `wait` by however long the address iterator takes to yield, by one address's timeout, or by up to
    /// 500 ms closing on a server that isn't reading.
    pub fn poll(&mut self, wait: Duration) -> Option<Event> {
        // Overflow means no deadline.
        let deadline = Instant::now().checked_add(wait);

        loop {
            if let Some(event) = self.events.pop_front() {
                return Some(event);
            }
            if self.signal.take_woken() {
                return None;
            }

            let backlogged =
                matches!(&self.state, State::Open(connection) if !connection.writable());
            let idle = self.step();

            if !self.events.is_empty() {
                continue;
            }
            if backlogged && self.writable() {
                return None;
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return None;
            }
            if idle {
                match &self.state {
                    State::Open(connection) => connection.wait(deadline),
                    State::Waiting { until } => self.signal.wait(Some(
                        deadline.map_or(*until, |deadline| deadline.min(*until)),
                    )),
                    _ => self.signal.wait(deadline),
                }
            }
        }
    }

    /// Leaves the namespace and closes the connection.
    pub fn close(mut self) {
        if self.connected {
            self.send(wire::disconnect(&self.namespace));
        }
        if let State::Open(connection) = std::mem::replace(&mut self.state, State::Idle) {
            connection.close();
        }
    }

    /// Does one thing that's due. Returns true if there was nothing to do.
    fn step(&mut self) -> bool {
        match &mut self.state {
            State::Open(connection) => {
                // Before each read, as Node runs due timers before reading, so neither steady traffic nor a ping
                // queued while the caller wasn't polling can revive a connection past its deadline.
                if connection.ping_expired() {
                    self.on_close(DisconnectReason::PingTimeout);
                    return false;
                }
                match connection.read() {
                    Read::Lost(reason) => self.on_close(reason),
                    Read::Nothing => return true,
                    Read::Handled => {}
                    Read::Packet(inbound) => self.on_packet(inbound),
                }
                false
            }
            State::Opening { .. } => {
                self.step_opening();
                false
            }
            State::Waiting { until } if Instant::now() >= *until => {
                self.open();
                false
            }
            State::Waiting { .. } | State::Idle => true,
        }
    }

    fn open(&mut self) {
        self.was_writable = false;
        self.state = State::Opening {
            candidates: None,
            next: None,
            error: None,
            acks: Vec::new(),
        };
    }

    fn step_opening(&mut self) {
        let State::Opening {
            candidates,
            next,
            error,
            acks,
        } = &mut self.state
        else {
            return;
        };

        if let Some(address) = next.take() {
            match Connection::open(
                address,
                &self.options.path,
                self.options.timeout,
                &self.signal,
            ) {
                Ok(connection) => {
                    let acks = std::mem::take(acks);
                    self.on_open(connection, acks);
                }
                Err(reason) => {
                    log::trace!("{address}: {reason}");
                    *error = Some(format!("{address}: {reason}"));
                }
            }
            return;
        }

        match candidates.get_or_insert_with(&mut self.addresses).next() {
            Some(address) => *next = Some(address),
            None => {
                let error = error
                    .take()
                    .unwrap_or_else(|| "no address to connect to".into());
                self.state = State::Idle;
                self.events
                    .push_back(Event::ConnectError(ConnectError::Transport(error)));
                self.reconnect();
            }
        }
    }

    /// <https://github.com/socketio/socket.io-protocol#connection-to-a-namespace>
    fn on_open(&mut self, connection: Connection, acks: Vec<String>) {
        self.state = State::Open(Box::new(connection));
        self.connection += 1;
        self.backoff.reset();
        for ack in acks {
            self.send(ack);
        }
        self.send(wire::connect(&self.namespace, self.auth.as_ref()));
    }

    /// Reference: `Manager.onclose`. Resets the backoff, so a long-lived connection reconnects quickly. `Socket` gets
    /// `disconnect` joined or not, preceded by `connect_error` for a transport error if it hadn't joined.
    fn on_close(&mut self, reason: DisconnectReason) {
        self.drop_connection();
        if let DisconnectReason::TransportError(error) = &reason
            && !self.connected
        {
            self.events
                .push_back(Event::ConnectError(ConnectError::Transport(error.clone())));
        }
        self.connected = false;
        self.events.push_back(Event::Disconnect(reason));
        self.backoff.reset();
        self.reconnect();
    }

    fn reconnect(&mut self) {
        if self.options.reconnection && self.joining {
            self.state = State::Waiting {
                until: Instant::now() + self.backoff.duration(),
            };
        }
    }

    /// The server ended the namespace. With no other namespace, the reference client closes the connection and stays
    /// down.
    fn end(&mut self, event: Event) {
        self.drop_connection();
        self.joining = false;
        self.connected = false;
        self.events.push_back(event);
    }

    fn drop_connection(&mut self) {
        if let State::Open(connection) = std::mem::replace(&mut self.state, State::Idle) {
            self.was_writable = connection.writable();
            connection.close();
        }
    }

    fn on_packet(&mut self, inbound: Inbound) {
        match inbound {
            // Reference: `Socket.onpacket` ignores other namespaces.
            Inbound::Connected { namespace, .. }
            | Inbound::Disconnected { namespace }
            | Inbound::ConnectError { namespace, .. }
            | Inbound::Event { namespace, .. }
                if namespace != self.namespace => {}

            // The reference reports it and keeps waiting.
            Inbound::Connected {
                with_sid: false, ..
            } => self
                .events
                .push_back(Event::ConnectError(ConnectError::UnsupportedServer)),
            Inbound::Connected { .. } => {
                let State::Open(connection) = &self.state else {
                    return;
                };
                let address = connection.address;
                self.connected = true;
                // Reference: `Socket.emitBuffered`. Received, then sent, then `connect`.
                self.events.extend(self.received_early.drain(..));
                for frame in std::mem::take(&mut self.sent_early) {
                    self.send(frame);
                }
                self.events.push_back(Event::Connect { address });
            }
            Inbound::ConnectError { message, data, .. } => {
                self.end(Event::ConnectError(ConnectError::Refused { message, data }));
            }
            // https://github.com/socketio/socket.io-protocol#disconnection-from-a-namespace
            Inbound::Disconnected { .. } => {
                self.end(Event::Disconnect(DisconnectReason::ServerDisconnect));
            }
            Inbound::Event { name, args, id, .. } => {
                let event = Event::Message {
                    name,
                    args,
                    ack: id.map(|id| Responder {
                        id,
                        connection: self.connection,
                    }),
                };
                if self.connected {
                    self.events.push_back(event);
                } else {
                    self.received_early.push(event);
                }
            }
            Inbound::Ignored(packet) => log::debug!("Ignoring {packet}"),
        }
    }

    /// A failure surfaces on a later read, after what the server sent before it.
    fn send(&mut self, text: String) {
        if let State::Open(connection) = &mut self.state {
            connection.send(text);
        }
    }
}
