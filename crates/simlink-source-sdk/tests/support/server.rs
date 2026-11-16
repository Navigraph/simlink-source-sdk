//! A minimal Simlink for the SDK's tests: the server side of Engine.IO v4 and Socket.IO v5 over a websocket.
//!
//! Written from the protocol spec, separately from the SDK's client codec, so one misreading can't pass on both
//! sides. It validates no payloads; the tests check what it receives against the manifest.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::http::StatusCode;
use tungstenite::{Message, WebSocket};

use super::manifest::MANIFEST;

const POLL: Duration = Duration::from_millis(10);
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(2);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);
const BIND_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
const ANSWER_TIMEOUT: Duration = Duration::from_secs(2);
// socket.io's defaults, long enough that tests which don't ask for pings never see one.
const PING_INTERVAL: Duration = Duration::from_secs(25);
const PING_TIMEOUT: Duration = Duration::from_secs(20);

/// One Simlink on a free loopback port. Dropping it closes every connection, and fails the test if a source
/// broke the protocol.
pub struct Server {
    address: SocketAddr,
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
}

struct Shared {
    state: Mutex<State>,
    connections: Mutex<Vec<JoinHandle<()>>>,
    changed: Condvar,
    stop: AtomicBool,
    /// Stops pings and everything else going out, as a hung Simlink would.
    silent: AtomicBool,
    ping_interval: Duration,
    ping_timeout: Duration,
    next_sid: AtomicU64,
}

#[derive(Default)]
struct State {
    outboxes: Vec<Sender<Outgoing>>,
    /// Descriptors of joined sources, in order.
    joins: VecDeque<Value>,
    /// Every event received, in order.
    events: Vec<(String, Value)>,
    /// How many of each channel's events `wait_for_event` has returned.
    seen: HashMap<String, usize>,
    refusals: VecDeque<Value>,
    leaves: usize,
    pongs: usize,
    violations: Vec<String>,
}

enum Outgoing {
    Command {
        name: String,
        input: Value,
        reply: Sender<Value>,
    },
    Event {
        name: String,
        payload: Value,
    },
    Leave,
    Close,
}

impl Server {
    pub fn start() -> Server {
        Server::with_heartbeat(PING_INTERVAL, PING_TIMEOUT)
    }

    /// Listens on Simlink's default port, the first one discovery probes, so an unmodified source finds it.
    pub fn on_simlink_port() -> Result<Server, String> {
        let port = MANIFEST["x-connection"]["defaultPort"].as_u64().unwrap() as u16;
        // The previous test's connections can hold the port for a moment.
        let deadline = Instant::now() + BIND_RETRY_TIMEOUT;
        loop {
            match TcpListener::bind((Ipv4Addr::LOCALHOST, port)) {
                Ok(listener) => return Ok(Server::listen(listener, PING_INTERVAL, PING_TIMEOUT)),
                Err(_) if Instant::now() < deadline => std::thread::sleep(POLL),
                Err(error) => {
                    return Err(format!(
                        "port {port} is taken, so discovery would not reach this server; is Simlink running? ({error})"
                    ));
                }
            }
        }
    }

    /// Pings every `interval` and advertises `timeout`, so the SDK's heartbeat runs within a test.
    pub fn with_heartbeat(interval: Duration, timeout: Duration) -> Server {
        Server::listen(
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap(),
            interval,
            timeout,
        )
    }

    fn listen(listener: TcpListener, interval: Duration, timeout: Duration) -> Server {
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let shared = Arc::new(Shared {
            state: Mutex::default(),
            connections: Mutex::default(),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            silent: AtomicBool::new(false),
            ping_interval: interval,
            ping_timeout: timeout,
            next_sid: AtomicU64::new(1),
        });
        let accept = {
            let shared = shared.clone();
            std::thread::spawn(move || accept(&shared, &listener))
        };

        Server {
            address,
            shared,
            accept: Some(accept),
        }
    }

    pub fn address(&self) -> String {
        self.address.to_string()
    }

    /// The descriptor of the next source to join.
    pub fn wait_for_join(&self, timeout: Duration) -> Option<Value> {
        self.wait(timeout, |state| state.joins.pop_front())
    }

    /// The next payload on `event` this method has not returned yet.
    pub fn wait_for_event(&self, event: &str, timeout: Duration) -> Option<Value> {
        self.wait(timeout, |state| {
            let seen = state.seen.get(event).copied().unwrap_or(0);
            let (_, payload) = state
                .events
                .iter()
                .filter(|(name, _)| name == event)
                .nth(seen)
                .cloned()?;
            state.seen.insert(event.to_owned(), seen + 1);
            Some(payload)
        })
    }

    /// Every payload received on `event` so far.
    pub fn events(&self, event: &str) -> Vec<Value> {
        self.lock()
            .events
            .iter()
            .filter(|(name, _)| name == event)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    /// Waits until `count` sources have left the namespace.
    pub fn wait_for_leaves(&self, count: usize, timeout: Duration) -> bool {
        self.wait(timeout, |state| (state.leaves >= count).then_some(()))
            .is_some()
    }

    /// Waits until the sources have answered `count` pings.
    pub fn wait_for_pongs(&self, count: usize, timeout: Duration) -> bool {
        self.wait(timeout, |state| (state.pongs >= count).then_some(()))
            .is_some()
    }

    /// Sends `command` to the latest source and returns its acknowledgement, or `None` without one.
    pub fn command(&self, command: &str, input: Value) -> Option<Value> {
        let (reply, answer) = mpsc::channel();
        let outgoing = Outgoing::Command {
            name: command.to_owned(),
            input,
            reply,
        };
        self.lock().outboxes.last()?.send(outgoing).ok()?;
        answer.recv_timeout(ANSWER_TIMEOUT).ok()
    }

    /// Sends `payload` on the error channel, as Simlink does for invalid data.
    pub fn send_error(&self, payload: Value) {
        let name = MANIFEST["x-connection"]["errorChannel"]
            .as_str()
            .unwrap()
            .to_owned();
        self.send_all(|| Outgoing::Event {
            name: name.clone(),
            payload: payload.clone(),
        });
    }

    /// Refuses the next handshake with `error`, a handshake error from the manifest.
    pub fn refuse_next_join(&self, error: Value) {
        self.lock().refusals.push_back(error);
    }

    /// Removes every source from the namespace but keeps its connection open.
    pub fn leave_all(&self) {
        self.send_all(|| Outgoing::Leave);
    }

    /// Closes every connection, as Simlink does when it shuts down.
    pub fn close_all(&self) {
        self.send_all(|| Outgoing::Close);
    }

    /// Stops pings and frames on open connections, which stay open. New connections still get their handshake.
    pub fn go_silent(&self) {
        self.shared.silent.store(true, Ordering::Relaxed);
    }

    /// Waits for the sources' last frames, then returns the violations `Drop` would otherwise fail the test with.
    pub fn finish(&self) -> Vec<String> {
        self.drain();
        std::mem::take(&mut self.lock().violations)
    }

    /// Lets connections end on their own, after the sources' last frames such as the leave.
    fn drain(&self) {
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        while Instant::now() < deadline
            && self
                .shared
                .connections
                .lock()
                .unwrap()
                .iter()
                .any(|c| !c.is_finished())
        {
            std::thread::sleep(POLL);
        }
    }

    fn send_all(&self, outgoing: impl Fn() -> Outgoing) {
        for outbox in &self.lock().outboxes {
            let _ = outbox.send(outgoing());
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap()
    }

    fn wait<T>(
        &self,
        timeout: Duration,
        mut take: impl FnMut(&mut State) -> Option<T>,
    ) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        loop {
            if let Some(found) = take(&mut state) {
                return Some(found);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            state = self.shared.changed.wait_timeout(state, left).unwrap().0;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Sources are usually dropped first.
        self.drain();
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        for connection in std::mem::take(&mut *self.shared.connections.lock().unwrap()) {
            let _ = connection.join();
        }
        // A connection thread can't fail the test itself, so its findings surface here.
        let violations = std::mem::take(&mut self.lock().violations);
        if !std::thread::panicking() {
            assert!(
                violations.is_empty(),
                "the source broke the protocol:\n{}",
                violations.join("\n")
            );
        }
    }
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.lock().unwrap());
        self.changed.notify_all();
    }
}

fn accept(shared: &Arc<Shared>, listener: &TcpListener) {
    while !shared.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                let connection = {
                    let shared = shared.clone();
                    std::thread::spawn(move || serve(&shared, stream))
                };
                shared.connections.lock().unwrap().push(connection);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

/// One source connection, from the websocket upgrade until either side leaves.
fn serve(shared: &Shared, stream: TcpStream) {
    stream.set_nonblocking(false).unwrap();
    // A client that never upgrades, such as a port probe, must not park this thread.
    stream.set_read_timeout(Some(UPGRADE_TIMEOUT)).unwrap();
    // Like Simlink, answer only on the published Socket.IO path.
    let path = format!(
        "{}/",
        MANIFEST["x-connection"]["socketIo"]["path"]
            .as_str()
            .unwrap()
    );
    // tungstenite's callback signature fixes the error type.
    #[allow(clippy::result_large_err)]
    let on_path = |request: &Request, response: Response| {
        if request.uri().path() == path {
            Ok(response)
        } else {
            let mut refusal = ErrorResponse::new(None);
            *refusal.status_mut() = StatusCode::NOT_FOUND;
            Err(refusal)
        }
    };
    let Ok(socket) = tungstenite::accept_hdr(stream, on_path) else {
        return;
    };
    // As in the SDK: a Windows socket is unreliable once a read times out.
    socket.get_ref().set_nonblocking(true).unwrap();

    let sid = format!("test-{}", shared.next_sid.fetch_add(1, Ordering::Relaxed));
    let mut connection = Connection {
        shared,
        socket,
        sid,
        outbox: None,
        pending: HashMap::new(),
        next_ack: 0,
    };
    connection.run();
}

struct Connection<'a> {
    shared: &'a Shared,
    socket: WebSocket<TcpStream>,
    sid: String,
    /// Commands and frames queued for the joined source.
    outbox: Option<Receiver<Outgoing>>,
    pending: HashMap<u64, Sender<Value>>,
    next_ack: u64,
}

impl Connection<'_> {
    fn run(&mut self) {
        let millis = |duration: Duration| duration.as_millis() as u64;
        let open = json!({
            "sid": self.sid,
            "upgrades": [],
            "pingInterval": millis(self.shared.ping_interval),
            "pingTimeout": millis(self.shared.ping_timeout),
        });
        self.send(&format!("0{open}"));
        let mut next_ping = Instant::now() + self.shared.ping_interval;

        while !self.shared.stop.load(Ordering::Relaxed) {
            let silent = self.shared.silent.load(Ordering::Relaxed);
            if !silent && Instant::now() >= next_ping {
                self.send("2");
                next_ping += self.shared.ping_interval;
            }

            while let Some(outgoing) = self
                .outbox
                .as_ref()
                .filter(|_| !silent)
                .and_then(|outbox| outbox.try_recv().ok())
            {
                match outgoing {
                    Outgoing::Command { name, input, reply } => {
                        self.next_ack += 1;
                        self.pending.insert(self.next_ack, reply);
                        self.send(&packet(
                            '2',
                            &format!("{}{}", self.next_ack, json!([name, input])),
                        ));
                    }
                    Outgoing::Event { name, payload } => {
                        self.send(&packet('2', &json!([name, payload]).to_string()))
                    }
                    Outgoing::Leave => {
                        self.send(&packet('1', ""));
                        self.outbox = None;
                    }
                    Outgoing::Close => return self.close(),
                }
            }

            let _ = self.socket.flush();
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    if !self.receive(&text) {
                        return self.close();
                    }
                }
                Ok(Message::Close(_)) => return,
                Ok(_) => {}
                Err(tungstenite::Error::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(POLL)
                }
                Err(_) => return,
            }
        }
        self.close();
    }

    /// Handles one Engine.IO packet from the source. Returns `false` once the source closes the connection.
    fn receive(&mut self, text: &str) -> bool {
        // Simlink drops the connection on a larger message.
        let limit = MANIFEST["x-connection"]["socketIo"]["maxPayloadBytes"]
            .as_u64()
            .unwrap();
        if text.len() as u64 > limit {
            self.violation(format!(
                "a {} byte message, over the {limit} byte limit",
                text.len()
            ));
            return false;
        }

        // Engine.IO: 1 close, 3 pong, 4 message. Socket.IO messages here are `4<type><namespace>,<ack id><json>`.
        match text.chars().next() {
            Some('1') => return false,
            Some('3') => {
                self.shared.update(|state| state.pongs += 1);
                return true;
            }
            Some('4') => {}
            _ => {
                self.violation(format!("unexpected Engine.IO packet {text}"));
                return true;
            }
        }

        let namespace = MANIFEST["x-connection"]["namespace"].as_str().unwrap();
        let (kind, rest) = text[1..].split_at_checked(1).unwrap_or(("", ""));
        let Some(rest) = rest
            .strip_prefix(namespace)
            .and_then(|rest| rest.strip_prefix(','))
        else {
            self.violation(format!("packet outside {namespace}: {text}"));
            return true;
        };
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        let id = rest[..digits].parse::<u64>().ok();
        let body: Option<Value> = serde_json::from_str(&rest[digits..]).ok();

        match (kind, id, body) {
            ("0", _, auth) => self.join(auth.unwrap_or(Value::Null)),
            ("1", _, _) => {
                self.shared.update(|state| state.leaves += 1);
                self.outbox = None;
            }
            ("2", None, Some(Value::Array(args))) => {
                match (args.first().cloned(), args.get(1).cloned()) {
                    (Some(Value::String(name)), payload) => {
                        self.shared.update(|state| {
                            state.events.push((name, payload.unwrap_or(Value::Null)))
                        });
                    }
                    _ => self.violation(format!("event without a name: {text}")),
                }
            }
            ("3", Some(id), Some(Value::Array(mut args))) => match self.pending.remove(&id) {
                Some(reply) => {
                    let _ = reply.send(if args.is_empty() {
                        Value::Null
                    } else {
                        args.swap_remove(0)
                    });
                }
                None => self.violation(format!("acknowledgement for unknown command {id}: {text}")),
            },
            _ => self.violation(format!("unexpected Socket.IO packet {text}")),
        }

        true
    }

    /// Admits the source, or refuses it with socket.io's `CONNECT_ERROR` and the handshake error in `data`.
    fn join(&mut self, descriptor: Value) {
        let (outbox, inbox) = mpsc::channel();
        let mut refusal = None;
        self.shared
            .update(|state| match state.refusals.pop_front() {
                Some(error) => refusal = Some(error),
                None => {
                    state.outboxes.push(outbox);
                    state.joins.push_back(descriptor);
                }
            });

        match refusal {
            Some(error) => {
                let code = error["code"].as_str().unwrap_or_default();
                let error = json!({ "message": format!("Refused: {code}"), "data": error });
                self.send(&packet('4', &error.to_string()));
            }
            None => {
                self.outbox = Some(inbox);
                self.send(&packet('0', &json!({ "sid": self.sid }).to_string()));
            }
        }
    }

    fn violation(&self, violation: String) {
        self.shared.update(|state| state.violations.push(violation));
    }

    fn send(&mut self, frame: &str) {
        // The source may already be gone; the read loop notices.
        let _ = self.socket.send(Message::text(frame));
    }

    fn close(&mut self) {
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}

/// A Socket.IO packet of `kind` on the source namespace, inside an Engine.IO message.
fn packet(kind: char, body: &str) -> String {
    format!(
        "4{kind}{},{body}",
        MANIFEST["x-connection"]["namespace"].as_str().unwrap()
    )
}
