//! Behavior beyond the protocol, after socket.io-client's `Manager` and `Socket`. Each test names what it checks.

mod support;

use std::time::{Duration, Instant};

use serde_json::json;
use socket_io::{Client, ConnectError, DisconnectReason, Event, Frame, Options};
use support::*;

fn event(name: &str) -> Frame {
    Frame::event(NS, name, vec![]).unwrap()
}

mod manager {
    use super::*;

    /// `Manager.onclose` then `reconnect`, keeping the auth.
    #[test]
    fn reconnects_and_rejoins_after_losing_the_connection() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept(r#"40/custom,{"token":"abc"}"#);
            peer.close();
            listener.accept().accept(r#"40/custom,{"token":"abc"}"#);
        });

        let mut client = joined_with(address, NS, Some(json!({ "token": "abc" })), fast());
        assert_disconnect(next(&mut client), DisconnectReason::TransportClose);
        assert_connect(next(&mut client), address);
        server.join().unwrap();
    }

    /// `Manager.maybeReconnectOnOpen`. Each attempt asks for the addresses again.
    #[test]
    fn retries_until_an_address_answers() {
        let (address, server) = serve(|mut listener| listener.accept().accept("40/custom,"));
        let dead = dead_address();
        let mut attempts = 0;

        let mut client = Client::new(
            move || {
                attempts += 1;
                Box::new(std::iter::once(if attempts == 1 { dead } else { address }))
            },
            NS,
            None,
            fast(),
        )
        .unwrap();
        assert!(matches!(
            next(&mut client),
            Event::ConnectError(ConnectError::Transport(_))
        ));
        assert_connect(next(&mut client), address);
        server.join().unwrap();
    }

    #[test]
    fn stays_closed_without_reconnection() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            peer.close();
            listener.accept().accept("40/custom,");
        });

        let mut client = joined(address, NS, None);
        assert_disconnect(next(&mut client), DisconnectReason::TransportClose);
        assert_quiet(&mut client, Duration::from_millis(300));

        client.join();
        assert_connect(next(&mut client), address);
        server.join().unwrap();
    }

    /// `Manager.reconnect` with backo2.
    #[test]
    fn backs_off_while_no_address_answers() {
        // Attempt start times, since a refused connect takes about 2 s on Windows.
        let attempts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let started = attempts.clone();
        let mut client = Client::new(
            move || {
                started.lock().unwrap().push(Instant::now());
                Box::new(std::iter::once(dead_address()))
            },
            NS,
            None,
            Options {
                reconnection_delay: Duration::from_millis(100),
                reconnection_delay_max: Duration::from_secs(5),
                ..Options::default()
            },
        )
        .unwrap();
        for _ in 0..4 {
            assert!(matches!(
                next(&mut client),
                Event::ConnectError(ConnectError::Transport(_))
            ));
        }
        let failed = Instant::now();
        while attempts.lock().unwrap().len() < 5 {
            assert!(
                failed.elapsed() < Duration::from_secs(5),
                "no fifth attempt"
            );
            client.poll(Duration::from_millis(10));
        }
        let delay = attempts.lock().unwrap()[4] - failed;
        // 800 ms, less half for jitter; without doubling, 150 ms at most.
        assert!(delay >= Duration::from_millis(350), "{delay:?}");
    }

    /// `Manager.onclose` and `onreconnect`.
    #[test]
    fn starts_the_backoff_over_after_a_session() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            peer.close();
            listener.accept().accept("40/custom,");
        });
        let dead = dead_address();
        let mut attempts = 0;

        let mut client = Client::new(
            move || {
                attempts += 1;
                Box::new(std::iter::once(if attempts <= 3 { dead } else { address }))
            },
            NS,
            None,
            Options {
                reconnection_delay: Duration::from_millis(200),
                reconnection_delay_max: Duration::from_secs(5),
                ..Options::default()
            },
        )
        .unwrap();
        for _ in 0..3 {
            assert!(matches!(
                next(&mut client),
                Event::ConnectError(ConnectError::Transport(_))
            ));
        }
        assert_connect(next(&mut client), address);
        assert_disconnect(next(&mut client), DisconnectReason::TransportClose);
        let closed = Instant::now();
        assert_connect(next(&mut client), address);
        // 200 ms, give or take half; without the reset at least 800 ms.
        assert!(
            closed.elapsed() < Duration::from_millis(750),
            "{:?}",
            closed.elapsed()
        );
        server.join().unwrap();
    }

    /// `Manager.onreconnect`: the session opening counts, even if the server then refuses the namespace.
    #[test]
    fn starts_the_backoff_over_when_a_session_opens() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            peer.send(r#"44/custom,{"message":"Not authorized"}"#);
            peer.wait_closed();
        });
        let dead = dead_address();
        // Attempt start times, since a refused connect takes about 2 s on Windows.
        let attempts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let started = attempts.clone();

        let mut client = Client::new(
            move || {
                let mut started = started.lock().unwrap();
                started.push(Instant::now());
                Box::new(std::iter::once(if started.len() == 4 {
                    address
                } else {
                    dead
                }))
            },
            NS,
            None,
            Options {
                reconnection_delay: Duration::from_millis(200),
                reconnection_delay_max: Duration::from_secs(5),
                ..Options::default()
            },
        )
        .unwrap();
        for _ in 0..3 {
            assert!(matches!(
                next(&mut client),
                Event::ConnectError(ConnectError::Transport(_))
            ));
        }
        assert!(matches!(
            next(&mut client),
            Event::ConnectError(ConnectError::Refused { .. })
        ));
        server.join().unwrap();

        client.join();
        assert!(matches!(
            next(&mut client),
            Event::ConnectError(ConnectError::Transport(_))
        ));
        let failed = Instant::now();
        assert!(matches!(
            next(&mut client),
            Event::ConnectError(ConnectError::Transport(_))
        ));
        let delay = attempts.lock().unwrap()[5] - failed;
        // 200 ms, give or take half; without the reset at least 800 ms.
        assert!(delay < Duration::from_millis(750), "{delay:?}");
    }

    /// `Manager.open`'s timeout.
    #[test]
    fn gives_up_on_an_address_that_never_opens_a_session() {
        // Upgrades, then never sends the open packet.
        let (silent, stalled) = serve(|mut listener| {
            let mut peer = listener.accept();
            let _ = peer.0.read();
        });
        let (address, server) = serve(|mut listener| listener.accept().accept("40/custom,"));

        let started = Instant::now();
        let mut client = Client::new(
            move || Box::new([silent, address].into_iter()),
            NS,
            None,
            Options {
                timeout: Duration::from_millis(200),
                ..once()
            },
        )
        .unwrap();
        assert_connect(next(&mut client), address);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        stalled.join().unwrap();
        server.join().unwrap();
    }

    /// `Manager.open`'s timeout, which pings don't reset.
    #[test]
    fn gives_up_on_an_address_that_only_pings() {
        let (pinging, flood) = serve(|mut listener| {
            let mut peer = listener.accept();
            // Until the client hangs up, capped so a failure doesn't hang the test.
            let until = Instant::now() + Duration::from_secs(3);
            while Instant::now() < until
                && peer
                    .0
                    .send(tungstenite::Message::Ping(Vec::new().into()))
                    .is_ok()
            {}
        });
        let (address, server) = serve(|mut listener| listener.accept().accept("40/custom,"));

        let started = Instant::now();
        let mut client = Client::new(
            move || Box::new([pinging, address].into_iter()),
            NS,
            None,
            Options {
                timeout: Duration::from_millis(200),
                ..once()
            },
        )
        .unwrap();
        assert_connect(next(&mut client), address);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        flood.join().unwrap();
        server.join().unwrap();
    }

    /// `Manager`'s `opts.path || "/socket.io"`.
    #[test]
    fn takes_the_default_path_for_an_empty_one() {
        // The server asserts the path.
        let (address, server) = serve(|mut listener| listener.accept().accept("40/custom,"));

        joined_with(
            address,
            NS,
            None,
            Options {
                path: String::new(),
                ..once()
            },
        );
        server.join().unwrap();
    }

    #[test]
    fn opens_the_first_address_that_answers() {
        let (address, server) = serve(|mut listener| listener.accept().accept("40/custom,"));
        let dead = dead_address();

        let mut client = Client::new(
            move || Box::new([dead, address].into_iter()),
            NS,
            None,
            once(),
        )
        .unwrap();
        assert_connect(next(&mut client), address);
        server.join().unwrap();
    }
}

mod socket {
    use super::*;

    /// `Socket.emit` with `volatile`: dropped until joined.
    #[test]
    fn drops_events_until_joined() {
        let (joining, joined) = std::sync::mpsc::channel();
        let (opened, open) = std::sync::mpsc::channel();
        let (address, server) = serve(move |mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            joining.send(()).unwrap();
            open.recv().unwrap();
            peer.assert_nothing_sent(Duration::from_millis(200));
            peer.send(r#"40/custom,{"sid":"a"}"#);
            assert_eq!(peer.recv(), r#"42/custom,["later"]"#);
        });

        let mut client = client(address, NS, None, once());
        // Not open yet.
        client.emit(event("dropped"));
        quiet_until(&mut client, &joined);
        // Open, but not joined.
        client.emit(event("dropped"));
        opened.send(()).unwrap();
        assert_connect(next(&mut client), address);
        client.emit(event("later"));
        finish(client, server);
    }

    /// `Socket.onevent` and `emitBuffered`: delivered before `connect`.
    #[test]
    fn holds_events_received_before_connecting() {
        let (ack, acked) = std::sync::mpsc::channel();
        let (address, server) = serve(move |mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            peer.send(r#"42/custom,0["early"]"#);
            acked.recv().unwrap();
            peer.send(r#"40/custom,{"sid":"a"}"#);
            assert_eq!(peer.recv(), "43/custom,0[]");
        });

        let mut client = client(address, NS, None, once());
        assert_quiet(&mut client, Duration::from_millis(300));
        ack.send(()).unwrap();
        let Event::Message {
            ack: Some(responder),
            ..
        } = next(&mut client)
        else {
            panic!("expected an event awaiting an ack");
        };
        assert_connect(next(&mut client), address);
        // It came over this connection, before the server's answer.
        client.ack(responder, vec![]);
        finish(client, server);
    }

    /// engine.io-client's ping timer fires before what's queued is read, so a ping that arrived while the caller
    /// wasn't polling doesn't revive the connection.
    #[test]
    fn times_out_before_reading_a_late_ping() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept_after(SHORT_HEARTBEAT, "40/custom,");
            peer.send("2");
            peer.wait_closed();
        });

        let mut client = joined(address, NS, None);
        // Past pingInterval + pingTimeout, without polling.
        std::thread::sleep(Duration::from_millis(600));
        let polled = Instant::now();
        assert_disconnect(next(&mut client), DisconnectReason::PingTimeout);
        // At once; reading the ping first would put it off by another 500 ms.
        assert!(
            polled.elapsed() < Duration::from_millis(250),
            "{:?}",
            polled.elapsed()
        );
        drop(client);
        server.join().unwrap();
    }

    /// `Socket.emit` and engine.io-client's `_hasPingExpired`.
    #[test]
    fn closes_instead_of_emitting_once_the_ping_is_overdue() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept_after(SHORT_HEARTBEAT, "40/custom,");
            peer.wait_closed();
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            peer.assert_nothing_sent(Duration::from_millis(200));
        });

        let mut client = joined_with(address, NS, None, fast());
        // Past pingInterval + pingTimeout, without polling.
        std::thread::sleep(Duration::from_millis(600));
        client.emit(event("dropped"));
        assert_disconnect(next(&mut client), DisconnectReason::PingTimeout);
        assert_connect(next(&mut client), address);
        finish(client, server);
    }

    /// `Socket.onerror` then `onclose`.
    #[test]
    fn reports_a_transport_error_before_joining() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            // A websocket frame with a reserved opcode.
            std::io::Write::write_all(peer.0.get_mut(), &[0x83, 0x00]).unwrap();
            peer.wait_closed();
        });

        let mut client = client(address, NS, None, once());
        assert!(matches!(
            next(&mut client),
            Event::ConnectError(ConnectError::Transport(_))
        ));
        assert!(matches!(
            next(&mut client),
            Event::Disconnect(DisconnectReason::TransportError(_))
        ));
        server.join().unwrap();
    }

    /// `Socket.onevent` and `onconnect`: events received before the connect ack come before the next `connect`. Ack
    /// ids start over with each connection, so their answers are dropped.
    #[test]
    fn keeps_held_events_across_a_lost_connection() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            peer.send(r#"42/custom,0["early"]"#);
            peer.close();
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            peer.assert_nothing_sent(Duration::from_millis(200));
        });

        let mut client = client(address, NS, None, fast());
        assert!(matches!(
            next(&mut client),
            Event::Disconnect(DisconnectReason::TransportClose)
        ));
        let Event::Message {
            ack: Some(responder),
            ..
        } = next(&mut client)
        else {
            panic!("expected an event awaiting an ack");
        };
        assert_connect(next(&mut client), address);
        client.ack(responder, vec![]);
        finish(client, server);
    }

    /// `Socket.onpacket`. A v2 server answers this way; the client keeps waiting.
    #[test]
    fn reports_a_connect_answer_without_a_session_id() {
        let (proceed, proceeding) = std::sync::mpsc::channel();
        let (address, server) = serve(move |mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            peer.send("40/custom,");
            proceeding.recv().unwrap();
            peer.send(r#"40/custom,{"sid":"a"}"#);
        });

        let mut client = client(address, NS, None, fast());
        assert!(matches!(
            next(&mut client),
            Event::ConnectError(ConnectError::UnsupportedServer)
        ));
        assert_quiet(&mut client, Duration::from_millis(300));
        proceed.send(()).unwrap();
        assert_connect(next(&mut client), address);
        server.join().unwrap();
    }

    /// `Socket.onpacket` and `Manager._destroy`: the connection closes too, until joined again.
    #[test]
    fn ends_a_refused_namespace() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.send(support::OPEN);
            peer.recv();
            peer.send(r#"44/custom,{"message":"Not authorized","data":{"code":"NOPE"}}"#);
            peer.wait_closed();
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            assert_eq!(peer.recv(), r#"42/custom,["after"]"#);
        });

        let mut client = client(address, NS, None, fast());
        match next(&mut client) {
            Event::ConnectError(ConnectError::Refused { message, data }) => {
                assert_eq!(message, "Not authorized");
                assert_eq!(data, Some(json!({ "code": "NOPE" })));
            }
            other => panic!("{other:?}"),
        }
        assert_quiet(&mut client, Duration::from_millis(300));

        client.join();
        // The new connection isn't open yet, as in the reference.
        client.emit(event("dropped"));
        assert_connect(next(&mut client), address);
        client.emit(event("after"));
        finish(client, server);
    }

    /// `Socket.ondisconnect` and `Manager._destroy`: stays closed until joined again.
    #[test]
    fn ends_a_namespace_the_server_disconnected() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            peer.send("41/custom,");
            peer.wait_closed();
            listener.accept().accept("40/custom,");
        });

        let mut client = joined_with(address, NS, None, fast());
        assert_disconnect(next(&mut client), DisconnectReason::ServerDisconnect);
        assert_quiet(&mut client, Duration::from_millis(300));

        client.join();
        assert_connect(next(&mut client), address);
        server.join().unwrap();
    }
}

/// The client sleeps between socket activity, so these check what has to wake it. This client only.
mod waiting {
    use super::*;

    #[test]
    fn wakes_for_a_frame() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            std::thread::sleep(Duration::from_millis(200));
            peer.send(r#"42/custom,["late"]"#);
            peer.close();
        });

        let mut client = joined(address, NS, None);
        let polled = Instant::now();
        assert_message(
            client.poll(Duration::MAX).expect("an event"),
            "late",
            vec![],
        );
        assert!(
            polled.elapsed() < Duration::from_secs(2),
            "{:?}",
            polled.elapsed()
        );
        finish(client, server);
    }

    #[test]
    fn wakes_for_the_ping_deadline() {
        let (address, server) = serve(|mut listener| {
            let mut peer = listener.accept();
            peer.accept_after(SHORT_HEARTBEAT, "40/custom,");
            peer.wait_closed();
        });

        let mut client = joined(address, NS, None);
        let polled = Instant::now();
        assert_disconnect(
            client.poll(Duration::MAX).expect("an event"),
            DisconnectReason::PingTimeout,
        );
        // pingInterval + pingTimeout is 500 ms.
        assert!(
            polled.elapsed() < Duration::from_secs(2),
            "{:?}",
            polled.elapsed()
        );
        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn wakes_for_a_waker() {
        let (closing, closed) = std::sync::mpsc::channel();
        let (address, server) = serve(move |mut listener| {
            let mut peer = listener.accept();
            peer.accept("40/custom,");
            closed.recv().unwrap();
            peer.close();
        });

        let mut client = joined(address, NS, None);
        let waker = client.waker();
        let polled = Instant::now();
        let woken = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            waker.wake();
        });
        assert!(client.poll(Duration::MAX).is_none());
        assert!(
            polled.elapsed() < Duration::from_secs(2),
            "{:?}",
            polled.elapsed()
        );
        woken.join().unwrap();

        closing.send(()).unwrap();
        finish(client, server);
    }
}
