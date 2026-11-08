//! One joined `/v1/source` namespace over one websocket.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::{Message, WebSocket};

use crate::protocol::{NAMESPACE, SOCKET_IO_PATH};
use crate::wire::{self, Inbound};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
// Windows takes about 2 s to refuse a closed loopback port; a listening one answers at once.
const LOOPBACK_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const CLOSE_TIMEOUT: Duration = Duration::from_millis(500);
// Short enough to stay responsive, long enough not to keep a sim's CPU awake.
const READ_RETRY: Duration = Duration::from_millis(5);

#[derive(Debug)]
pub enum JoinError {
    /// Nothing that speaks the Simlink protocol answered.
    Unreachable(String),
    /// Simlink refused the descriptor.
    Rejected { code: String, message: String },
}

#[derive(Debug)]
pub enum Received {
    Frame(Inbound),
    Idle,
    Lost(String),
}

pub struct Session {
    socket: WebSocket<TcpStream>,
    heartbeat: Duration,
    alive_until: Instant,
    /// Frames are waiting for the socket, so Simlink is reading slower than the source sends.
    backlogged: bool,
}

impl Session {
    pub fn join(address: SocketAddr, descriptor: &Value) -> Result<Session, JoinError> {
        let unreachable =
            |error: &dyn std::fmt::Display| JoinError::Unreachable(format!("{address}: {error}"));
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

        let url = format!("ws://{address}{SOCKET_IO_PATH}/?EIO=4&transport=websocket");
        let (socket, _) = tungstenite::client(url.as_str(), stream).map_err(|e| unreachable(&e))?;
        // A Windows socket is unreliable once a read times out, so poll without timeouts from here.
        socket
            .get_ref()
            .set_nonblocking(true)
            .map_err(|e| unreachable(&e))?;

        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut session = Session {
            socket,
            heartbeat: HANDSHAKE_TIMEOUT,
            alive_until: deadline,
            backlogged: false,
        };

        let Received::Frame(Inbound::Open {
            ping_interval,
            ping_timeout,
        }) = session.receive_until(deadline)
        else {
            return Err(unreachable(&"no Engine.IO open packet"));
        };
        session.heartbeat = ping_interval + ping_timeout;
        session
            .send(&wire::connect(NAMESPACE, descriptor))
            .map_err(|e| unreachable(&e))?;

        loop {
            // Checked per frame, so a peer that keeps sending other traffic can't hold the join open.
            if Instant::now() >= deadline {
                return Err(unreachable(&"no namespace answer"));
            }
            match session.receive_until(deadline) {
                Received::Frame(Inbound::Connected) => break,
                Received::Frame(Inbound::ConnectError { message, data }) => {
                    let code = data
                        .as_ref()
                        .and_then(|data| data.get("code"))
                        .and_then(Value::as_str);
                    return Err(JoinError::Rejected {
                        code: code.unwrap_or_default().to_owned(),
                        message,
                    });
                }
                Received::Frame(Inbound::Ping) => {
                    session.send(wire::PONG).map_err(|e| unreachable(&e))?
                }
                Received::Frame(_) | Received::Idle => {}
                Received::Lost(reason) => return Err(unreachable(&reason)),
            }
        }

        session.alive_until = Instant::now() + session.heartbeat;

        Ok(session)
    }

    /// Waits up to `wait` for one frame. Engine.IO pings are answered here.
    pub fn poll(&mut self, wait: Duration) -> Received {
        match self.receive_until(Instant::now() + wait) {
            Received::Frame(Inbound::Ping) => match self.send(wire::PONG) {
                Ok(()) => Received::Idle,
                Err(error) => Received::Lost(error),
            },
            Received::Idle if Instant::now() > self.alive_until => {
                Received::Lost("heartbeat timed out".into())
            }
            received => received,
        }
    }

    pub fn send(&mut self, frame: &str) -> Result<(), String> {
        match self.socket.send(Message::text(frame)) {
            // Queued; later polls flush it.
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                self.backlogged = true;
                Ok(())
            }
            result => result.map_err(|error| error.to_string()),
        }
    }

    pub fn backlogged(&self) -> bool {
        self.backlogged
    }

    /// Leaves the namespace and closes the socket, giving a stalled Simlink a short while to take the leave.
    pub fn close(mut self) {
        let _ = self.send(&wire::disconnect(NAMESPACE));
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

    fn receive_until(&mut self, deadline: Instant) -> Received {
        loop {
            match self.receive() {
                Received::Idle if Instant::now() < deadline => std::thread::sleep(READ_RETRY),
                received => return received,
            }
        }
    }

    fn receive(&mut self) -> Received {
        match self.socket.flush() {
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                self.backlogged = true
            }
            Err(error) => return Received::Lost(error.to_string()),
            Ok(()) => self.backlogged = false,
        }

        match self.socket.read() {
            Ok(Message::Text(text)) => {
                self.alive_until = Instant::now() + self.heartbeat;

                match wire::decode(&text, NAMESPACE) {
                    Ok(Inbound::Close) => Received::Lost("Simlink closed the connection".into()),
                    Ok(inbound) => Received::Frame(inbound),
                    Err(error) => {
                        log::warn!("Ignoring {error}");
                        Received::Idle
                    }
                }
            }
            Ok(Message::Close(_)) => Received::Lost("Simlink closed the connection".into()),
            Ok(_) => Received::Idle,
            Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                Received::Idle
            }
            Err(error) => Received::Lost(error.to_string()),
        }
    }
}
