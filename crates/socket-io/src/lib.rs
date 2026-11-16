//! A Socket.IO v5 client over Engine.IO v4 websockets, for one namespace and JSON payloads.
//!
//! The wire layer reads the full protocol except HTTP long-polling, so sessions start on websocket, which the
//! Engine.IO spec allows. A frame the protocol doesn't allow closes the connection.
//!
//! - <https://github.com/socketio/socket.io-protocol>
//! - <https://github.com/socketio/engine.io-protocol>
//!
//! The client joins one namespace, sends events without acks, answers the server's acks, and drops binary packets.
//! Where the protocols leave behavior open, reconnection included, it follows the reference client, socket.io-client.
//!
//! Everything runs on the caller's thread inside [`Client::poll`], which sleeps until the socket, a deadline or a
//! [`Waker`] needs it.

mod backoff;
mod client;
mod engine;
mod waker;
mod wire;

use std::net::SocketAddr;
use std::time::Duration;

use serde_json::Value;

pub use backoff::Backoff;
pub use client::{Addresses, Client};
pub use waker::Waker;

/// How a [`Client`] connects and reconnects. Defaults match the reference client.
#[derive(Clone, Debug)]
pub struct Options {
    /// Path the server serves Socket.IO at. Empty means `/socket.io`.
    pub path: String,
    /// Reconnect after a lost connection or failed attempt. When off, retry with [`Client::join`].
    pub reconnection: bool,
    /// Initial reconnect delay, with jitter. Doubles per attempt that opens no session, and resets once one opens or
    /// closes.
    pub reconnection_delay: Duration,
    /// Cap on the reconnect delay.
    pub reconnection_delay_max: Duration,
    /// Time from connecting to an address to its Engine.IO open packet. Must not be zero; [`Duration::MAX`] waits
    /// forever. Doesn't cover the namespace's `CONNECT` answer, which the heartbeat bounds.
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            path: "/socket.io".into(),
            reconnection: true,
            reconnection_delay: Duration::from_secs(1),
            reconnection_delay_max: Duration::from_secs(5),
            timeout: Duration::from_secs(20),
        }
    }
}

/// What [`Client::poll`] returns.
#[derive(Debug)]
pub enum Event {
    /// The namespace was joined. Events the server sent before its `CONNECT` answer arrive just before this.
    Connect { address: SocketAddr },
    /// Joining failed. Retried with [`Options::reconnection`] unless refused.
    ConnectError(ConnectError),
    /// The connection closed or the server disconnected the namespace, joined or not; a refusal reports only
    /// [`ConnectError::Refused`]. Reconnects with [`Options::reconnection`] unless the server ended it.
    Disconnect(DisconnectReason),
    /// An event from the server. `ack` is set if it asked for one.
    Message {
        name: String,
        args: Vec<Value>,
        ack: Option<Responder>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConnectError {
    /// No address opened a session, or a transport error came before joining, which [`Event::Disconnect`] follows.
    Transport(String),
    /// The server answered `CONNECT` without a session id, as v2 servers do. The client keeps waiting.
    UnsupportedServer,
    /// The server sent `CONNECT_ERROR`. Ends the namespace until [`Client::join`].
    Refused {
        message: String,
        data: Option<Value>,
    },
}

/// Why a connection closed. Displays as the reference client's reason string.
#[derive(Clone, Debug, PartialEq)]
pub enum DisconnectReason {
    /// The server disconnected the namespace. Ends it until [`Client::join`].
    ServerDisconnect,
    /// No ping within `pingInterval + pingTimeout`.
    PingTimeout,
    /// The server closed the connection.
    TransportClose,
    /// The connection failed, or the server sent a malformed Engine.IO packet or websocket frame.
    TransportError(String),
    /// The server sent a Socket.IO packet the protocol doesn't allow.
    ParseError(String),
}

impl std::fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisconnectReason::ServerDisconnect => write!(f, "io server disconnect"),
            DisconnectReason::PingTimeout => write!(f, "ping timeout"),
            DisconnectReason::TransportClose => write!(f, "transport close"),
            DisconnectReason::TransportError(error) => write!(f, "transport error: {error}"),
            DisconnectReason::ParseError(error) => write!(f, "parse error: {error}"),
        }
    }
}

/// An event name the reference parser rejects.
#[derive(Clone, Debug, PartialEq)]
pub struct ReservedName(pub String);

impl std::fmt::Display for ReservedName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "\"{}\" is a reserved event name", self.0)
    }
}

impl std::error::Error for ReservedName {}

/// An encoded event, ready for [`Client::emit`].
#[derive(Clone, Debug)]
pub struct Frame(String);

impl Frame {
    /// Encodes an event. `namespace` must be the client's.
    pub fn event(namespace: &str, name: &str, args: Vec<Value>) -> Result<Frame, ReservedName> {
        wire::event(namespace, name, args)
            .map(Frame)
            .ok_or_else(|| ReservedName(name.to_owned()))
    }

    /// Size in bytes, as a server's message limit counts it.
    pub fn size(&self) -> usize {
        self.0.len()
    }
}

/// Answers one server event with [`Client::ack`].
#[derive(Debug)]
pub struct Responder {
    id: u64,
    connection: u64,
}
