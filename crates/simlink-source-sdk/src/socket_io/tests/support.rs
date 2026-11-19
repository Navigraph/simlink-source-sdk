//! A scripted server: each test plays the server's half of an exchange frame by frame.

#![allow(dead_code)]

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;
use crate::socket_io::{Client, DisconnectReason, Event, Options};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::{Message, WebSocket};

pub const NS: &str = "/custom";
/// The Engine.IO spec's example open packet. The server never pings, so its heartbeat outlasts any test.
pub const OPEN: &str = r#"0{"sid":"lv_VI97HAXpY6yYWAAAC","upgrades":["websocket"],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}"#;
/// The official suites' heartbeat.
pub const SHORT_HEARTBEAT: &str = r#"0{"sid":"lv_VI97HAXpY6yYWAAAC","upgrades":[],"pingInterval":300,"pingTimeout":200,"maxPayload":1000000}"#;

pub struct Listener(TcpListener);

impl Listener {
    pub fn accept(&mut self) -> Peer {
        self.upgrade(None).expect("accepted")
    }

    /// Refuses the next upgrade with `body`, as the reference server does for invalid query parameters.
    pub fn refuse(&mut self, body: &'static str) {
        assert!(self.upgrade(Some(body)).is_none());
    }

    // tungstenite's upgrade callback sets the error type.
    #[allow(clippy::result_large_err)]
    fn upgrade(&mut self, refusal: Option<&'static str>) -> Option<Peer> {
        let (stream, _) = self.0.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let callback = |request: &Request, response: Response| {
            assert_eq!(request.uri(), "/socket.io/?EIO=4&transport=websocket");
            let Some(refusal) = refusal else {
                return Ok(response);
            };
            let mut refusal = ErrorResponse::new(Some(refusal.into()));
            *refusal.status_mut() = StatusCode::BAD_REQUEST;
            Err(refusal)
        };
        tungstenite::accept_hdr(stream, callback).ok().map(Peer)
    }
}

/// The server's end of one connection.
pub struct Peer(pub WebSocket<TcpStream>);

impl Peer {
    pub fn send(&mut self, text: &str) {
        self.0.send(Message::text(text)).unwrap();
    }

    pub fn send_binary(&mut self, bytes: &[u8]) {
        self.0.send(Message::binary(bytes.to_vec())).unwrap();
    }

    /// The next text frame from the client.
    pub fn recv(&mut self) -> String {
        loop {
            match self.0.read().unwrap() {
                Message::Text(text) => return text.to_string(),
                Message::Binary(_) => panic!("expected text, got binary"),
                Message::Close(_) => panic!("the client closed the connection"),
                _ => {}
            }
        }
    }

    /// Closes with a close frame, as the reference server does.
    pub fn close(&mut self) {
        self.0.close(None).unwrap();
        self.0.flush().unwrap();
    }

    /// Asserts the client sends nothing for `wait`. Nonblocking, since a Windows socket is unreliable after a read
    /// timeout.
    pub fn assert_nothing_sent(&mut self, wait: Duration) {
        self.0.get_ref().set_nonblocking(true).unwrap();
        std::thread::sleep(wait);
        match self.0.read() {
            Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => panic!("expected nothing, got {other:?}"),
        }
        self.0.get_ref().set_nonblocking(false).unwrap();
    }

    /// Waits for the client to hang up, so it reads everything sent before the connection closes.
    pub fn wait_closed(&mut self) {
        loop {
            match self.0.read() {
                Ok(Message::Close(_)) => return,
                Ok(_) => {}
                Err(error) => panic!("expected the client to close the connection: {error}"),
            }
        }
    }

    /// Opens the session and accepts the client into its namespace, checking its `CONNECT` packet.
    pub fn accept(&mut self, connect: &str) {
        self.accept_after(OPEN, connect);
    }

    /// `accept`, opening the session with `open`.
    pub fn accept_after(&mut self, open: &str, connect: &str) {
        self.send(open);
        assert_eq!(self.recv(), connect);
        let namespace = match connect.strip_prefix("40/") {
            Some(rest) => &connect[2..3 + rest.find(',').unwrap() + 1],
            None => "",
        };
        self.send(&format!(r#"40{namespace}{{"sid":"wZX3oN0bSVIhsaknAAAI"}}"#));
    }
}

/// Serves the client's connections with `script`.
pub fn serve(script: impl FnOnce(Listener) + Send + 'static) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || script(Listener(listener)));
    (address, server)
}

/// An address nothing listens on.
pub fn dead_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

/// Without reconnection.
pub fn once() -> Options {
    Options {
        reconnection: false,
        ..fast()
    }
}

/// Reconnecting quickly.
pub fn fast() -> Options {
    Options {
        reconnection_delay: Duration::from_millis(20),
        reconnection_delay_max: Duration::from_millis(100),
        ..Options::default()
    }
}

/// A client for the server at `address`, joining `namespace` with `auth`.
pub fn client(
    address: SocketAddr,
    namespace: &str,
    auth: Option<Value>,
    options: Options,
) -> Client {
    let auth = auth.map(|auth| auth.as_object().cloned().expect("auth is an object"));
    Client::new(
        move || Box::new(std::iter::once(address)),
        namespace,
        auth,
        options,
    )
    .unwrap()
}

/// The next event, failing after a few seconds without one.
pub fn next(client: &mut Client) -> Event {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Some(event) = client.poll(Duration::from_millis(50)) {
            return event;
        }
    }
    panic!("nothing happened");
}

/// Polls the client, expecting nothing to happen, until the server signals on `signal`.
pub fn quiet_until(client: &mut Client, signal: &std::sync::mpsc::Receiver<()>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while signal.try_recv().is_err() {
        assert!(Instant::now() < deadline, "the server never signaled");
        assert_quiet(client, Duration::from_millis(10));
    }
}

pub fn assert_quiet(client: &mut Client, wait: Duration) {
    if let Some(event) = client.poll(wait) {
        panic!("expected nothing, got {event:?}");
    }
}

/// A client joined to `namespace` with `auth`, without reconnection.
pub fn joined(address: SocketAddr, namespace: &str, auth: Option<Value>) -> Client {
    joined_with(address, namespace, auth, once())
}

pub fn joined_with(
    address: SocketAddr,
    namespace: &str,
    auth: Option<Value>,
    options: Options,
) -> Client {
    let mut client = client(address, namespace, auth, options);
    assert_connect(next(&mut client), address);
    client
}

/// Polls until the server's script ends and drops the connection, then surfaces the script's failures.
pub fn finish(mut client: Client, server: JoinHandle<()>) {
    loop {
        match next(&mut client) {
            Event::Disconnect(reason) => {
                break assert_eq!(reason, DisconnectReason::TransportClose);
            }
            Event::ConnectError(error) => panic!("{error:?}"),
            _ => {}
        }
    }
    server.join().unwrap();
}

/// JSON arguments.
pub fn args(json: Value) -> Vec<Value> {
    let Value::Array(values) = json else {
        panic!("arguments are an array");
    };
    values
}

pub fn assert_connect(event: Event, address: SocketAddr) {
    assert!(
        matches!(&event, Event::Connect { address: joined } if *joined == address),
        "{event:?}"
    );
}

pub fn assert_message(event: Event, name: &str, args: Vec<Value>) {
    match event {
        Event::Message {
            name: got_name,
            args: got_args,
            ..
        } => {
            assert_eq!(got_name, name);
            assert_eq!(got_args, args);
        }
        other => panic!("expected {name}, got {other:?}"),
    }
}

pub fn assert_disconnect(event: Event, reason: DisconnectReason) {
    assert!(
        matches!(&event, Event::Disconnect(got) if *got == reason),
        "{event:?}"
    );
}

/// Closed because the client rejected a frame, not because the server went away.
pub fn assert_parse_error(event: Event) {
    assert!(
        matches!(&event, Event::Disconnect(DisconnectReason::ParseError(_))),
        "{event:?}"
    );
}
