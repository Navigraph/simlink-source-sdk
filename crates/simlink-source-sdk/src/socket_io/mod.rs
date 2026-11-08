//! A Socket.IO v5 client over Engine.IO v4, websocket transport only, joined to one namespace. Nothing here knows
//! about Simlink.

mod wire;

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::{Message, WebSocket};

use wire::Inbound;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
// Windows takes about 2 s to refuse a closed loopback port; a listening one answers at once.
const LOOPBACK_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
// Short enough to stay responsive, long enough not to keep a sim's CPU awake.
const READ_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub enum ConnectError {
    /// Nothing that speaks Socket.IO answered.
    Unreachable(String),
    /// The server refused to join the namespace, with its `connect_error` message and data.
    Refused {
        message: String,
        data: Option<Value>,
    },
}

#[derive(Debug)]
pub enum Received {
    /// An event, with the id to acknowledge when the server awaits an answer.
    Event {
        name: String,
        args: Vec<Value>,
        ack: Option<u64>,
    },
    Idle,
    Lost(String),
}

/// An encoded event, ready for a socket joined to its namespace.
#[derive(Debug)]
pub struct Frame(String);

impl Frame {
    pub fn event(namespace: &str, name: &str, payload: &Value) -> Frame {
        Frame(wire::event(namespace, name, payload))
    }

    /// The size a Socket.IO server's message limit applies to, in bytes.
    pub fn size(&self) -> usize {
        self.0.len()
    }
}

/// What one read from the socket gave.
enum Read {
    Packet(Inbound),
    Idle,
    Lost(String),
}

pub struct Socket {
    socket: WebSocket<TcpStream>,
    namespace: String,
    heartbeat: Duration,
    alive_until: Instant,
    /// Frames are waiting for the socket, so the server is reading slower than we send.
    backlogged: bool,
}

impl Socket {
    /// Connects to the Socket.IO server at `address`, serving at `path`, and joins `namespace` with `auth`.
    pub fn connect(
        address: SocketAddr,
        path: &str,
        namespace: &str,
        auth: &Value,
    ) -> Result<Socket, ConnectError> {
        let unreachable = |error: &dyn std::fmt::Display| {
            ConnectError::Unreachable(format!("{address}: {error}"))
        };
        let connect_timeout = if address.ip().is_loopback() {
            LOOPBACK_CONNECT_TIMEOUT
        } else {
            CONNECT_TIMEOUT
        };

        let stream =
            TcpStream::connect_timeout(&address, connect_timeout).map_err(|e| unreachable(&e))?;
        stream.set_nodelay(true).map_err(|e| unreachable(&e))?;
        stream
            .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
            .map_err(|e| unreachable(&e))?;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .map_err(|e| unreachable(&e))?;

        let url = format!("ws://{address}{path}/?EIO=4&transport=websocket");
        let (socket, _) = tungstenite::client(url.as_str(), stream).map_err(|e| unreachable(&e))?;
        // A Windows socket is unreliable once a read times out, so poll without timeouts from here.
        socket
            .get_ref()
            .set_nonblocking(true)
            .map_err(|e| unreachable(&e))?;

        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut socket = Socket {
            socket,
            namespace: namespace.to_owned(),
            heartbeat: HANDSHAKE_TIMEOUT,
            alive_until: deadline,
            backlogged: false,
        };

        let Read::Packet(Inbound::Open {
            ping_interval,
            ping_timeout,
        }) = socket.read_until(deadline)
        else {
            return Err(unreachable(&"no Engine.IO open packet"));
        };
        socket.heartbeat = ping_interval + ping_timeout;
        socket
            .send_text(&wire::connect(namespace, auth))
            .map_err(|e| unreachable(&e))?;

        loop {
            // Checked per frame, so a peer that keeps sending other traffic can't hold the join open.
            if Instant::now() >= deadline {
                return Err(unreachable(&"no namespace answer"));
            }
            match socket.read_until(deadline) {
                Read::Packet(Inbound::Connected) => break,
                Read::Packet(Inbound::ConnectError { message, data }) => {
                    return Err(ConnectError::Refused { message, data });
                }
                Read::Packet(Inbound::Ping) => {
                    socket.send_text(wire::PONG).map_err(|e| unreachable(&e))?
                }
                Read::Packet(_) | Read::Idle => {}
                Read::Lost(reason) => return Err(unreachable(&reason)),
            }
        }

        socket.alive_until = Instant::now() + socket.heartbeat;

        Ok(socket)
    }

    /// Waits up to `wait` for one event. Engine.IO pings are answered here.
    pub fn poll(&mut self, wait: Duration) -> Received {
        match self.read_until(Instant::now() + wait) {
            Read::Packet(Inbound::Event { name, args, ack }) => Received::Event { name, args, ack },
            Read::Packet(Inbound::Ping) => match self.send_text(wire::PONG) {
                Ok(()) => Received::Idle,
                Err(error) => Received::Lost(error),
            },
            Read::Packet(Inbound::Disconnected) => {
                Received::Lost("the server disconnected the namespace".into())
            }
            Read::Idle if Instant::now() > self.alive_until => {
                Received::Lost("heartbeat timed out".into())
            }
            Read::Packet(_) | Read::Idle => Received::Idle,
            Read::Lost(reason) => Received::Lost(reason),
        }
    }

    pub fn send(&mut self, frame: &Frame) -> Result<(), String> {
        self.send_text(&frame.0)
    }

    pub fn ack(&mut self, id: u64, value: &Value) -> Result<(), String> {
        self.send_text(&wire::ack(&self.namespace, id, value))
    }

    pub fn backlogged(&self) -> bool {
        self.backlogged
    }

    /// Leaves the namespace and closes the socket, giving a stalled server a short while to take the leave.
    pub fn close(mut self) {
        let _ = self.send_text(&wire::disconnect(&self.namespace));
        let _ = self.socket.close(None);
        let deadline = Instant::now() + CLOSE_TIMEOUT;
        while Instant::now() < deadline {
            match self.socket.flush() {
                Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(READ_RETRY)
                }
                _ => return,
            }
        }
    }

    fn send_text(&mut self, text: &str) -> Result<(), String> {
        match self.socket.send(Message::text(text)) {
            // Queued; later polls flush it.
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                self.backlogged = true;
                Ok(())
            }
            result => result.map_err(|error| error.to_string()),
        }
    }

    fn read_until(&mut self, deadline: Instant) -> Read {
        loop {
            match self.read() {
                Read::Idle if Instant::now() < deadline => std::thread::sleep(READ_RETRY),
                read => return read,
            }
        }
    }

    fn read(&mut self) -> Read {
        match self.socket.flush() {
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                self.backlogged = true
            }
            Err(error) => return Read::Lost(error.to_string()),
            Ok(()) => self.backlogged = false,
        }

        match self.socket.read() {
            Ok(Message::Text(text)) => {
                self.alive_until = Instant::now() + self.heartbeat;

                match wire::decode(&text, &self.namespace) {
                    Ok(Inbound::Close) => Read::Lost("the server closed the connection".into()),
                    Ok(inbound) => Read::Packet(inbound),
                    Err(error) => {
                        log::warn!("Ignoring {error}");
                        Read::Idle
                    }
                }
            }
            Ok(Message::Close(_)) => Read::Lost("the server closed the connection".into()),
            Ok(_) => Read::Idle,
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                Read::Idle
            }
            Err(error) => Read::Lost(error.to_string()),
        }
    }
}
