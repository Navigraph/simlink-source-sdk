//! The SDK's connection behavior against a test Simlink: handshake, events, commands, refusals and reconnects.

mod support;

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use simlink::*;
use support::manifest::{MANIFEST, issues};
use support::server::Server;

const WAIT: Duration = Duration::from_secs(5);

fn source(server: &Server, events: Vec<Event>) -> Source {
    let options = SourceOptions {
        id: "test-source".into(),
        display_name: "Test".into(),
        is_injecting_sim_traffic: false,
    };
    let source = Source::new(options, events);
    source.set_address(server.address()).unwrap();
    source
}

/// A refusal code, checked against the manifest so a rename there fails here.
fn refusal(code: &str) -> &str {
    let known: Vec<&Value> = MANIFEST["handshakeErrors"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .collect();
    assert!(
        known
            .iter()
            .any(|error| error["properties"]["code"]["const"] == code),
        "{code} is not in the manifest"
    );
    code
}

fn ownship(latitude: f64) -> TelemetryOwnship {
    serde_json::from_value(json!({
        "aircraft": {
            "position": { "latitude": latitude, "longitude": 17.9 },
            "altitude": { "true": 1200, "agl": 900, "indicated": 1200 },
            "speed": { "gs": 140, "ias": 135, "tas": 138 },
            "vertical_speed": 0,
            "heading": { "true": 32, "magnetic": 25 },
            "track": { "true": 31 },
            "attitude": { "bank": 0, "pitch": 0 },
            "turn_rate": 0,
            "transponder": "7000",
            "gforce": 1,
        },
        "simulation": { "paused": false, "slew": false },
    }))
    .unwrap()
}

/// Forwards every callback to a channel the test reads.
struct Forward<T>(Mutex<Sender<T>>);

fn forward<T>() -> (Arc<Forward<T>>, Receiver<T>) {
    let (sender, receiver) = mpsc::channel();
    (Arc::new(Forward(Mutex::new(sender))), receiver)
}

impl<T> Forward<T> {
    fn send(&self, value: T) {
        let _ = self.0.lock().unwrap().send(value);
    }
}

impl StateListener for Forward<ConnectionState> {
    fn changed(&self, state: ConnectionState) {
        self.send(state);
    }
}

impl RejectionListener for Forward<(String, String, String)> {
    fn rejected(&self, channel: String, code: String, message: String) {
        self.send((channel, code, message));
    }
}

impl AtisGetHandler for Forward<(AtisGetInput, Arc<AtisGetReply>)> {
    fn handle(&self, input: AtisGetInput, reply: Arc<AtisGetReply>) {
        self.send((input, reply));
    }
}

/// The next state `wanted` accepts, skipping others.
fn wait_for_state(
    states: &Receiver<ConnectionState>,
    wanted: fn(&ConnectionState) -> bool,
) -> ConnectionState {
    let deadline = Instant::now() + WAIT;
    loop {
        let state = states
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("state");
        if wanted(&state) {
            return state;
        }
    }
}

fn searching(state: &ConnectionState) -> bool {
    matches!(state, ConnectionState::Searching)
}

fn connected(state: &ConnectionState) -> bool {
    matches!(state, ConnectionState::Connected { .. })
}

fn rejected(state: &ConnectionState) -> bool {
    matches!(state, ConnectionState::Rejected { .. })
}

#[test]
fn joins_and_emits_valid_events() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryOwnship]);
    source.start().unwrap();

    let join = server.wait_for_join(WAIT).expect("the source joins");
    assert_eq!(issues(&MANIFEST["descriptor"], &join), Vec::<String>::new());
    assert_eq!(join["id"], "test-source");
    assert_eq!(
        join["capabilities"],
        json!({ "events": ["telemetry.ownship"], "commands": [] })
    );

    // Emits are dropped until the session is up, so keep emitting until one arrives.
    let deadline = Instant::now() + WAIT;
    let event = loop {
        source.emit_telemetry_ownship(ownship(59.6)).unwrap();
        if let Some(event) = server.wait_for_event("telemetry.ownship", Duration::from_millis(250))
        {
            break event;
        }
        assert!(Instant::now() < deadline, "no ownship arrived");
    };
    assert_eq!(event["aircraft"]["position"]["latitude"], 59.6);
}

#[test]
fn rejects_misuse() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryOwnship]);

    assert!(matches!(
        source.emit_telemetry_ownship(ownship(0.0)),
        Err(Error::WrongState { .. })
    ));
    source.start().unwrap();
    assert!(matches!(source.start(), Err(Error::WrongState { .. })));
    assert!(matches!(
        source.on_state(forward().0),
        Err(Error::WrongState { .. })
    ));
    assert!(matches!(
        source.emit_telemetry_traffic(vec![]),
        Err(Error::Undeclared { .. })
    ));
}

#[test]
fn answers_commands_now_and_later() {
    let server = Server::start();
    let source = source(&server, vec![]);
    let (atis, requests) = forward();
    source.on_atis_get(atis).unwrap();
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    // Answer from another thread, after the handler returned.
    let answering = std::thread::spawn(move || {
        let (input, reply) = requests.recv_timeout(WAIT).expect("the command arrives");
        assert_eq!(input.icao, "ESSA");
        reply.ok(vec![]).unwrap();
        assert!(matches!(
            reply.ok(vec![]),
            Err(Error::AlreadyAnswered { .. })
        ));

        let (_, reply) = requests
            .recv_timeout(WAIT)
            .expect("the second command arrives");
        reply.refuse(AtisGetError::AtisNotAvailable).unwrap();
    });

    let answers = [
        server.command("atis.get", json!({ "icao": "ESSA" })),
        server.command("atis.get", json!({ "icao": "ESSA" })),
    ];
    assert_eq!(
        answers,
        [
            Some(json!({ "ok": true, "value": [] })),
            Some(json!({ "ok": false, "error": "ATIS_NOT_AVAILABLE" }))
        ]
    );
    for answer in answers.iter().flatten() {
        assert_eq!(
            issues(&MANIFEST["commands"]["atis.get"]["result"], answer),
            Vec::<String>::new()
        );
    }
    answering.join().unwrap();
}

#[test]
fn refuses_payloads_over_the_message_limit() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryTraffic]);
    source.start().unwrap();

    let aircraft: TrafficPosition = serde_json::from_value(json!({
        "key": "a",
        "aircraft": {
            "identified_by": "type",
            "type": "A320",
            "position": { "latitude": 59.6, "longitude": 17.9 },
            "altitude": { "true": 1200 },
            "speed": { "gs": 140 },
            "heading": { "true": 32 },
            "attitude": {},
        },
    }))
    .unwrap();
    let limit = MANIFEST["connection"]["socketIo"]["maxPayloadBytes"]
        .as_u64()
        .unwrap() as usize;
    let size = serde_json::to_string(&aircraft).unwrap().len();

    assert!(matches!(
        source.emit_telemetry_traffic(vec![aircraft; limit / size + 1]),
        Err(Error::InvalidArgument { .. })
    ));
}

#[test]
fn relays_simlink_rejections() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryOwnship]);
    let (listener, rejections) = forward();
    source.on_rejected(listener).unwrap();
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    server.send_error(json!({ "event": "telemetry.ownship", "code": "INVALID_DATA", "message": "bad transponder" }));
    assert_eq!(
        rejections.recv_timeout(WAIT),
        Ok((
            "telemetry.ownship".into(),
            "INVALID_DATA".into(),
            "bad transponder".into()
        ))
    );
}

#[test]
fn sends_the_latest_payload_at_the_rate_limit() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryOwnship]);
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    // Warm up until one payload gets through, then emit far faster than the 5 Hz limit for one second.
    while server.events("telemetry.ownship").is_empty() {
        source.emit_telemetry_ownship(ownship(0.0)).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let before = server.events("telemetry.ownship").len();
    let started = Instant::now();
    let mut emitted = 0;
    while started.elapsed() < Duration::from_secs(1) {
        emitted += 1;
        source
            .emit_telemetry_ownship(ownship(emitted as f64))
            .unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(400));

    let events = server.events("telemetry.ownship");
    let sent = events.len() - before;
    assert!(
        (3..=7).contains(&sent),
        "{sent} payloads in a second at 5 Hz"
    );
    assert_eq!(
        events.last().unwrap()["aircraft"]["position"]["latitude"],
        json!(emitted as f64)
    );
}

#[test]
fn rejoins_after_being_removed_from_the_namespace() {
    let server = Server::start();
    let source = source(&server, vec![]);
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    source.start().unwrap();

    server.wait_for_join(WAIT).expect("the source joins");
    assert_eq!(
        wait_for_state(&states, connected),
        ConnectionState::Connected {
            address: server.address()
        }
    );

    // Only the namespace goes; the source must act on the leave itself.
    server.leave_all();
    wait_for_state(&states, searching);
    server.wait_for_join(WAIT).expect("the source joins again");
}

#[test]
fn reconnects_after_simlink_closes_the_connection() {
    let server = Server::start();
    let source = source(&server, vec![]);
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    source.start().unwrap();

    server.wait_for_join(WAIT).expect("the source joins");
    wait_for_state(&states, connected);
    server.close_all();
    wait_for_state(&states, searching);
    server.wait_for_join(WAIT).expect("the source joins again");
}

#[test]
fn answers_pings_and_notices_a_silent_simlink() {
    // A generous timeout, so a stalled CI runner can't drop the connection before the silence starts.
    let server = Server::with_heartbeat(Duration::from_millis(100), Duration::from_secs(1));
    let source = source(&server, vec![]);
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    source.start().unwrap();

    wait_for_state(&states, connected);
    assert!(
        server.wait_for_pongs(3, WAIT),
        "the source did not answer pings"
    );
    assert!(
        states.try_recv().is_err(),
        "the source reconnected while Simlink was answering"
    );

    server.go_silent();
    wait_for_state(&states, searching);
}

#[test]
fn leaves_when_dropped() {
    let server = Server::start();
    let source = source(&server, vec![]);
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    drop(source);
    assert!(
        server.wait_for_leaves(1, WAIT),
        "the source closed without leaving"
    );
}

#[test]
fn drops_promptly_while_searching() {
    let options = SourceOptions {
        id: "test-source".into(),
        display_name: "Test".into(),
        is_injecting_sim_traffic: false,
    };
    let source = Source::new(options, vec![]);
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    source.start().unwrap();
    wait_for_state(&states, searching);
    // Let discovery get under way.
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    drop(source);
    // At most one probe or the mDNS browse is still running.
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "dropping took {:?}",
        started.elapsed()
    );
}

#[test]
fn retries_a_refusal_that_can_clear() {
    let server = Server::start();
    server.refuse_next_join(refusal("HANDSHAKE_SOURCE_ID_IN_USE"));
    let source = source(&server, vec![]);
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    source.start().unwrap();

    let ConnectionState::Rejected { code, retrying, .. } = wait_for_state(&states, rejected) else {
        unreachable!()
    };
    assert_eq!(
        (code.as_str(), retrying),
        ("HANDSHAKE_SOURCE_ID_IN_USE", true)
    );
    server
        .wait_for_join(WAIT)
        .expect("the source joins on its next attempt");
}

#[test]
fn stops_after_an_invalid_identification() {
    let server = Server::start();
    server.refuse_next_join(refusal("HANDSHAKE_IDENTIFICATION_INVALID"));
    let source = source(&server, vec![]);
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    source.start().unwrap();

    let ConnectionState::Rejected { code, retrying, .. } = wait_for_state(&states, rejected) else {
        unreachable!()
    };
    assert_eq!(
        (code.as_str(), retrying),
        ("HANDSHAKE_IDENTIFICATION_INVALID", false)
    );
    // A retry reports Searching after the 1 s minimum backoff.
    assert!(
        states.recv_timeout(Duration::from_secs(2)).is_err(),
        "the source retried"
    );
}

#[test]
fn drops_replies_to_an_earlier_connection() {
    let server = Arc::new(Server::start());
    let source = source(&server, vec![]);
    let (atis, requests) = forward();
    source.on_atis_get(atis).unwrap();
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    let ask = || {
        let server = server.clone();
        std::thread::spawn(move || server.command("atis.get", json!({ "icao": "ESSA" })))
    };

    let asking = ask();
    let (_, stale) = requests.recv_timeout(WAIT).expect("the command arrives");
    server.leave_all();
    server.wait_for_join(WAIT).expect("the source joins again");
    assert_eq!(asking.join().unwrap(), None);

    // The new connection numbers its commands from 1 again, so a leaked reply would answer this one.
    let asking = ask();
    let (_, fresh) = requests.recv_timeout(WAIT).expect("the command arrives");
    stale.refuse(AtisGetError::AtisNotAvailable).unwrap();
    fresh.ok(vec![]).unwrap();
    assert_eq!(
        asking.join().unwrap(),
        Some(json!({ "ok": true, "value": [] }))
    );
}
