//! The SDK's connection behavior against a test Simlink: handshake, events, commands, refusals and reconnects.

mod support;

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use simlink_source_sdk::*;
use support::manifest::{MANIFEST, valid};
use support::server::Server;

const WAIT: Duration = Duration::from_secs(5);

/// A source that finds Simlink by discovery.
fn new_source(events: Vec<Event>) -> Source {
    let options = SourceOptions {
        id: "test-source".into(),
        display_name: "Test".into(),
    };
    Source::new(options, events)
}

/// A source that connects to `server`.
fn source(server: &Server, events: Vec<Event>) -> Source {
    let source = new_source(events);
    source.set_address(server.address()).unwrap();
    source
}

/// A refusal code, checked against the manifest so a rename there fails here.
fn refusal(code: &str) -> &str {
    let known = MANIFEST["handshakeErrors"]["oneOf"].as_array().unwrap();
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

impl RejectionListener for Forward<Rejection> {
    fn rejected(&self, rejection: Rejection) {
        self.send(rejection);
    }
}

impl AtisGetHandler for Forward<(AtisGetInput, Arc<AtisGetReply>)> {
    fn handle(&self, input: AtisGetInput, reply: Arc<AtisGetReply>) {
        self.send((input, reply));
    }
}

/// The source's connection states, from before it starts.
fn states(source: &Source) -> Receiver<ConnectionState> {
    let (listener, states) = forward();
    source.on_state(listener).unwrap();
    states
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

/// Emits ownships at `latitude` until one reaches `server`, since emits are dropped until the source joins.
fn emit_until_received(source: &Source, server: &Server, latitude: f64) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        source.emit_telemetry_ownship(ownship(latitude)).unwrap();
        if let Some(event) = server.wait_for_event("telemetry.ownship", Duration::from_millis(250))
        {
            return event;
        }
        assert!(Instant::now() < deadline, "no ownship arrived");
    }
}

#[test]
fn joins_and_emits_valid_events() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryOwnship]);
    source.start().unwrap();

    let join = server.wait_for_join(WAIT).expect("the source joins");
    valid("descriptor", &MANIFEST["descriptor"], &join).unwrap();
    assert_eq!(join["id"], "test-source");
    assert_eq!(
        join["capabilities"],
        json!({ "events": ["telemetry.ownship"], "commands": [] })
    );

    let event = emit_until_received(&source, &server, 59.6);
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
fn answers_and_refuses_commands_after_the_handler_returns() {
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
        valid(
            "answer",
            &MANIFEST["commands"]["atis.get"]["result"],
            answer,
        )
        .unwrap();
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
    let source = source(&server, vec![Event::TelemetryTraffic]);
    let (listener, rejections) = forward();
    source.on_rejected(listener).unwrap();
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    let frame = json!({
        "event": "telemetry.traffic",
        "code": "INVALID_DATA",
        "message": "bad traffic",
        "issues": [{ "code": "invalid_type", "message": "Required", "path": [3, "aircraft", "position"] }],
    });
    valid("error frame", &MANIFEST["errorFrame"], &frame).unwrap();
    server.send_error(frame);
    assert_eq!(
        rejections.recv_timeout(WAIT),
        Ok(Rejection {
            channel: "telemetry.traffic".into(),
            code: "INVALID_DATA".into(),
            message: "bad traffic".into(),
            issues: vec![RejectionIssue {
                path: "[3].aircraft.position".into(),
                message: "Required".into(),
            }],
        })
    );
}

#[test]
fn sends_the_latest_payload_at_the_rate_limit() {
    let server = Server::start();
    let source = source(&server, vec![Event::TelemetryOwnship]);
    source.start().unwrap();
    server.wait_for_join(WAIT).expect("the source joins");

    // Warm up until one payload gets through, then emit far faster than the 10 Hz limit for one second.
    emit_until_received(&source, &server, 0.0);
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
        (8..=12).contains(&sent),
        "{sent} payloads in a second at 10 Hz"
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
    let states = states(&source);
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
    let states = states(&source);
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
    let states = states(&source);
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
    let states = states(&source);
    source.start().unwrap();
    // A source still waiting for Simlink's answer has nothing to leave.
    wait_for_state(&states, connected);

    drop(source);
    assert!(
        server.wait_for_leaves(1, WAIT),
        "the source closed without leaving"
    );
}

#[test]
fn drops_promptly_while_searching() {
    let source = new_source(vec![]);
    let states = states(&source);
    source.start().unwrap();
    wait_for_state(&states, searching);
    // Let discovery get under way.
    std::thread::sleep(Duration::from_millis(300));

    let started = Instant::now();
    drop(source);
    // At most the loopback probes, the mDNS browse and probing what it found are still running.
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "dropping took {:?}",
        started.elapsed()
    );
}

#[test]
fn retries_a_refusal_that_can_clear() {
    let server = Server::start();
    let code = refusal("HANDSHAKE_SOURCE_ID_IN_USE");
    server.refuse_next_join(code);
    let source = source(&server, vec![]);
    let states = states(&source);
    source.start().unwrap();

    assert_eq!(
        wait_for_state(&states, rejected),
        ConnectionState::Rejected {
            code: code.into(),
            message: format!("Refused: {code}"),
            retrying: true,
        }
    );
    server
        .wait_for_join(WAIT)
        .expect("the source joins on its next attempt");
}

#[test]
fn drops_emits_while_disconnected() {
    let server = Server::start();
    server.refuse_next_join(refusal("HANDSHAKE_SOURCE_ID_IN_USE"));
    let source = source(&server, vec![Event::TelemetryOwnship]);
    let states = states(&source);
    source.start().unwrap();

    wait_for_state(&states, rejected);
    source.emit_telemetry_ownship(ownship(1.0)).unwrap();
    wait_for_state(&states, connected);
    assert!(
        server
            .wait_for_event("telemetry.ownship", Duration::from_millis(300))
            .is_none(),
        "an emit from while the source was refused arrived after it joined"
    );

    let event = emit_until_received(&source, &server, 2.0);
    assert_eq!(event["aircraft"]["position"]["latitude"], 2.0);
}

#[test]
fn stops_after_an_invalid_identification() {
    let server = Server::start();
    let code = refusal("HANDSHAKE_IDENTIFICATION_INVALID");
    server.refuse_next_join(code);
    let source = source(&server, vec![]);
    let states = states(&source);
    source.start().unwrap();

    assert_eq!(
        wait_for_state(&states, rejected),
        ConnectionState::Rejected {
            code: code.into(),
            message: format!("Refused: {code}"),
            retrying: false,
        }
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
