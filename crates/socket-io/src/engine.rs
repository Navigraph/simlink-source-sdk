//! One Engine.IO session over a websocket.
//! <https://github.com/socketio/engine.io-protocol>

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use polling::Event as Interest;
use tungstenite::error::ProtocolError;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

use crate::DisconnectReason;
use crate::waker::Signal;
use crate::wire::{self, Decoder, Engine, Inbound};

const NO_OPEN: &str = "no Engine.IO open packet";
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
// The default of Node's `ws`, which the reference client uses.
const MAX_MESSAGE_SIZE: usize = 100 << 20;
/// The socket's key in the poller.
const SOCKET: usize = 0;

pub enum Read {
    Nothing,
    /// A frame needing no attention, such as a ping or part of a binary packet.
    Handled,
    Packet(Inbound),
    Lost(DisconnectReason),
}

pub struct Connection {
    pub address: SocketAddr,
    socket: WebSocket<TcpStream>,
    decoder: Decoder,
    /// pingInterval + pingTimeout
    heartbeat: Duration,
    alive_until: Instant,
    /// Frames are queued, so the server reads slower than we send.
    backlogged: bool,
    /// Reported once what the server sent before it has been read.
    write_error: Option<DisconnectReason>,
    /// The socket is registered with its poller until the connection drops.
    signal: Arc<Signal>,
}

impl Connection {
    /// `timeout` runs from connecting to the open packet, like the reference `Manager`'s `timeout`.
    pub fn open(
        address: SocketAddr,
        path: &str,
        timeout: Duration,
        signal: &Arc<Signal>,
    ) -> Result<Connection, String> {
        // Overflow means no deadline.
        let deadline = Instant::now().checked_add(timeout);
        let remaining = || match deadline {
            // Never zero, which socket timeouts reject.
            Some(deadline) => deadline
                .saturating_duration_since(Instant::now())
                .max(Duration::from_millis(1)),
            None => timeout,
        };

        let stream = TcpStream::connect_timeout(&address, remaining()).map_err(text)?;
        stream.set_nodelay(true).map_err(text)?;
        stream.set_read_timeout(Some(remaining())).map_err(text)?;
        stream.set_write_timeout(Some(remaining())).map_err(text)?;

        // https://github.com/socketio/engine.io-protocol#handshake
        // Like the reference: empty means the default, and one trailing slash is dropped.
        let path = if path.is_empty() { "/socket.io" } else { path };
        let path = path.strip_suffix('/').unwrap_or(path);
        // Drop the IPv6 scope id, which URLs can't carry. The stream is already connected.
        let host = SocketAddr::new(address.ip(), address.port());
        let url = format!("ws://{host}{path}/?EIO=4&transport=websocket");
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_SIZE))
            .max_frame_size(None);
        let (socket, _) =
            tungstenite::client::client_with_config(url.as_str(), stream, Some(config))
                .map_err(text)?;
        // Windows sockets are unreliable after a read times out, so go nonblocking and wait on readiness from here.
        socket.get_ref().set_nonblocking(true).map_err(text)?;
        // SAFETY: `Drop` deletes the registration before the socket closes.
        unsafe {
            signal
                .poller
                .add(socket.get_ref(), Interest::readable(SOCKET))
        }
        .map_err(text)?;
        let mut connection = Connection {
            address,
            socket,
            decoder: Decoder::default(),
            heartbeat: Duration::ZERO,
            alive_until: Instant::now(),
            backlogged: false,
            write_error: None,
            signal: signal.clone(),
        };

        loop {
            // Checked every iteration, so a silent or pinging server can't hold the attempt open.
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(NO_OPEN.into());
            }
            match connection.socket.read() {
                Ok(Message::Text(frame)) => match wire::decode_engine(&frame) {
                    Ok(Engine::Open {
                        ping_interval,
                        ping_timeout,
                    }) => {
                        connection.set_heartbeat(ping_interval + ping_timeout);
                        return Ok(connection);
                    }
                    _ => return Err(NO_OPEN.into()),
                },
                // Tungstenite answers these, as `ws` does.
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(_) => return Err(NO_OPEN.into()),
                Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                    connection.wait_for(Interest::readable(SOCKET), deadline);
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    /// Blocks until the socket has something to read, or room to write while backlogged, a [`Waker`] fires, the ping
    /// deadline passes, or `until`.
    ///
    /// [`Waker`]: crate::Waker
    pub fn wait(&self, until: Option<Instant>) {
        let until = until.map_or(self.alive_until, |until| until.min(self.alive_until));
        self.wait_for(Interest::new(SOCKET, true, self.backlogged), Some(until));
    }

    fn wait_for(&self, interest: Interest, until: Option<Instant>) {
        // The poller reports each readiness once, so it's asked for again before every wait.
        if let Err(error) = self.signal.poller.modify(self.socket.get_ref(), interest) {
            log::warn!("Waiting on the socket failed: {error}");
        }
        self.signal.wait(until);
    }

    /// The server went `pingInterval + pingTimeout` without a ping.
    /// <https://github.com/socketio/engine.io-protocol#heartbeat>
    pub fn ping_expired(&self) -> bool {
        Instant::now() > self.alive_until
    }

    pub fn writable(&self) -> bool {
        !self.backlogged
    }

    pub fn send(&mut self, text: String) {
        self.send_message(Message::text(text));
    }

    /// Reads one websocket frame, answering Engine.IO pings.
    pub fn read(&mut self) -> Read {
        if self.write_error.is_none() {
            match self.socket.flush() {
                Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                    self.backlogged = true
                }
                Err(e) => self.write_error = Some(lost(e)),
                Ok(()) => self.backlogged = false,
            }
        }

        let decoded = match self.socket.read() {
            Ok(Message::Text(frame)) => match wire::decode_engine(&frame) {
                Ok(Engine::Ping) => {
                    // Timed by pings alone: https://github.com/socketio/engine.io-protocol#heartbeat
                    self.alive_until = Instant::now() + self.heartbeat;
                    self.send_message(Message::text(wire::PONG));
                    return Read::Handled;
                }
                // The reference parser yields an error packet, which the client treats as a transport error.
                Err(malformed) => {
                    return Read::Lost(DisconnectReason::TransportError(malformed.to_string()));
                }
                Ok(Engine::Message(packet)) => self.decoder.text(packet),
                Ok(Engine::Binary) => self.decoder.binary(),
                // Reference: Socket.onHandshake, which a later open packet runs again.
                Ok(Engine::Open {
                    ping_interval,
                    ping_timeout,
                }) => {
                    self.set_heartbeat(ping_interval + ping_timeout);
                    return Read::Handled;
                }
                Ok(Engine::Ignored) => return Read::Handled,
            },
            Ok(Message::Binary(_)) => self.decoder.binary(),
            Ok(Message::Close(_)) => return Read::Lost(DisconnectReason::TransportClose),
            Ok(_) => return Read::Handled,
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                return match self.write_error.take() {
                    Some(reason) => Read::Lost(reason),
                    None => Read::Nothing,
                };
            }
            Err(e) => return Read::Lost(lost(e)),
        };

        match decoded {
            Ok(Some(inbound)) => Read::Packet(inbound),
            Ok(None) => Read::Handled,
            // The stream can't be trusted after this.
            Err(malformed) => Read::Lost(DisconnectReason::ParseError(malformed.to_string())),
        }
    }

    /// Gives a stalled server up to `CLOSE_TIMEOUT` to take what's queued.
    pub fn close(mut self) {
        let _ = self.socket.close(None);
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        while Instant::now() < deadline {
            match self.socket.flush() {
                Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                    self.wait_for(Interest::writable(SOCKET), Some(deadline))
                }
                _ => return,
            }
        }
    }

    /// Restarts the ping deadline with a new `pingInterval + pingTimeout`.
    fn set_heartbeat(&mut self, heartbeat: Duration) {
        self.heartbeat = heartbeat;
        self.alive_until = Instant::now() + heartbeat;
    }

    /// Failures are kept for `read` to report.
    fn send_message(&mut self, message: Message) {
        if self.write_error.is_some() {
            return;
        }
        match self.socket.send(message) {
            // Queued; later reads flush it.
            Err(tungstenite::Error::Io(e)) if e.kind() == ErrorKind::WouldBlock => {
                self.backlogged = true
            }
            Err(e) => self.write_error = Some(lost(e)),
            Ok(()) => {}
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.signal.poller.delete(self.socket.get_ref());
    }
}

/// A server gone without a close frame counts as a transport close, since the reference client only sees the
/// websocket close.
fn lost(error: tungstenite::Error) -> DisconnectReason {
    match error {
        tungstenite::Error::ConnectionClosed
        | tungstenite::Error::AlreadyClosed
        | tungstenite::Error::Protocol(ProtocolError::ResetWithoutClosingHandshake) => {
            DisconnectReason::TransportClose
        }
        tungstenite::Error::Io(e)
            if matches!(
                e.kind(),
                ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
            ) =>
        {
            DisconnectReason::TransportClose
        }
        error => DisconnectReason::TransportError(error.to_string()),
    }
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}
