//! The [engine.io-protocol] and [socket.io-protocol] test suites, ported to the client's side: each test plays the
//! reference server's half of the suite's exchange and checks the client's. Tests follow the suites' order and link
//! the `it` they port; the socket.io suite's copies of the Engine.IO tests are ported once. Tests after "This client
//! only" have no counterpart. Left out:
//!
//! - long-polling and upgrades, since the client only speaks websocket;
//! - "should connect then disconnect from a custom namespace", since it joins two namespaces.
//!
//! [engine.io-protocol]: https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js
//! [socket.io-protocol]: https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js

mod support;

use std::time::{Duration, Instant};

use serde_json::json;
// The `socket_io` module below shadows the crate.
use ::socket_io::{ConnectError, DisconnectReason, Event, Frame};
use support::*;

/// Expects the client to close the connection over `frame`.
fn assert_rejects(frame: &'static str) {
    let (address, server) = serve(move |mut listener| {
        let mut peer = listener.accept();
        peer.accept("40");
        peer.send(frame);
        peer.wait_closed();
    });

    let mut client = joined(address, "/", None);
    assert_parse_error(next(&mut client));
    drop(client);
    server.join().unwrap();
}

/// Expects a failed connect when the server refuses the upgrade with `body`.
fn assert_refused_upgrade(body: &'static str) {
    let (address, server) = serve(move |mut listener| listener.refuse(body));

    let mut client = client(address, NS, None, once());
    assert!(matches!(
        next(&mut client),
        Event::ConnectError(ConnectError::Transport(_))
    ));
    server.join().unwrap();
}

mod engine_io {
    use super::*;

    mod handshake {
        use super::*;

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L112>
        #[test]
        fn successfully_opens_a_session() {
            let (address, server) = serve(|mut listener| {
                listener.accept().accept(r#"40/custom,{"token":"abc"}"#);
            });

            joined(address, NS, Some(json!({ "token": "abc" })));
            server.join().unwrap();
        }

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L139>
        #[test]
        fn fails_with_an_invalid_eio_query_parameter() {
            assert_refused_upgrade(r#"{"code":5,"message":"Unsupported protocol version"}"#);
        }

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L161>
        #[test]
        fn fails_with_an_invalid_transport_query_parameter() {
            assert_refused_upgrade(r#"{"code":0,"message":"Transport unknown"}"#);
        }

        // This client only.

        /// A websocket ping, as from a proxy, isn't the open packet.
        #[test]
        fn opens_a_session_after_a_websocket_ping() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.0
                    .send(tungstenite::Message::Ping(Vec::new().into()))
                    .unwrap();
                peer.accept("40/custom,");
            });

            joined(address, NS, None);
            server.join().unwrap();
        }
    }

    mod message {
        use super::*;

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L319>
        ///
        /// Exchanges the simplest Socket.IO packet.
        #[test]
        fn sends_and_receives_a_plain_text_packet() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                assert_eq!(peer.recv(), r#"42["hello"]"#);
                peer.send(r#"42["hello"]"#);
            });

            let mut client = joined(address, "/", None);
            client.emit(Frame::event("/", "hello", vec![]).unwrap());
            assert_message(next(&mut client), "hello", vec![]);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L337>
        ///
        /// Turned around: the client sends no binary, so the server sends an attachment, which the client drops.
        #[test]
        fn sends_and_receives_a_binary_packet() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"451-["hello",{"_placeholder":true,"num":0}]"#);
                peer.send_binary(&[1, 2, 3, 4]);
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L354>
        ///
        /// engine.io-parser decodes it as an error packet, a transport error to engine.io-client.
        #[test]
        fn closes_the_session_upon_invalid_packet_format() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send("abc");
                peer.wait_closed();
            });

            let mut client = joined(address, "/", None);
            assert!(matches!(
                next(&mut client),
                Event::Disconnect(DisconnectReason::TransportError(_))
            ));
            drop(client);
            server.join().unwrap();
        }

        // This client only.

        /// engine.io-parser takes a `b`-prefixed text frame as a base64 binary message.
        #[test]
        fn takes_a_base64_attachment() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"451-["hello",{"_placeholder":true,"num":0}]"#);
                peer.send("bAQIDBA==");
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }

        #[test]
        fn closes_the_session_upon_binary_outside_a_packet() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send_binary(&[1, 2, 3]);
                peer.wait_closed();
            });

            let mut client = joined(address, "/", None);
            assert_parse_error(next(&mut client));
            drop(client);
            server.join().unwrap();
        }
    }

    mod heartbeat {
        use super::*;

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L414>
        #[test]
        fn sends_ping_pong_packets() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept_after(SHORT_HEARTBEAT, "40");
                // At the suite's pingInterval, outlasting the heartbeat unless pings reset it.
                for _ in 0..3 {
                    std::thread::sleep(Duration::from_millis(300));
                    peer.send("2");
                    assert_eq!(peer.recv(), "3");
                }
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L432>
        #[test]
        fn closes_the_session_upon_ping_timeout() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept_after(SHORT_HEARTBEAT, "40");
                peer.wait_closed();
            });

            let opened_before = Instant::now();
            let mut client = joined(address, "/", None);
            assert_disconnect(next(&mut client), DisconnectReason::PingTimeout);
            // pingInterval + pingTimeout, with room for a slow runner.
            let waited = opened_before.elapsed();
            assert!(
                waited >= Duration::from_millis(500) && waited < Duration::from_millis(1500),
                "{waited:?}"
            );
            server.join().unwrap();
        }

        // This client only.

        /// Only pings reset the heartbeat.
        #[test]
        fn closes_the_session_upon_ping_timeout_despite_other_packets() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept_after(SHORT_HEARTBEAT, "40");
                // Until the client hangs up.
                while peer
                    .0
                    .send(tungstenite::Message::text(r#"42["tick"]"#))
                    .is_ok()
                {
                    std::thread::sleep(Duration::from_millis(100));
                }
            });

            let opened_before = Instant::now();
            let mut client = joined(address, "/", None);
            loop {
                assert!(opened_before.elapsed() < Duration::from_millis(1500));
                match next(&mut client) {
                    Event::Message { .. } => {}
                    event => break assert_disconnect(event, DisconnectReason::PingTimeout),
                }
            }
            drop(client);
            server.join().unwrap();
        }

        /// engine.io-client's `onHandshake` runs again on a later open packet, taking its heartbeat.
        #[test]
        fn takes_the_heartbeat_of_a_later_open_packet() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(SHORT_HEARTBEAT);
                peer.wait_closed();
            });

            let opened_before = Instant::now();
            let mut client = joined(address, "/", None);
            assert_disconnect(next(&mut client), DisconnectReason::PingTimeout);
            assert!(opened_before.elapsed() < Duration::from_millis(1500));
            drop(client);
            server.join().unwrap();
        }
    }

    mod close {
        use super::*;

        /// <https://github.com/socketio/engine.io-protocol/blob/f21de7b/test-suite/test-suite.js#L470>
        ///
        /// Turned around: the server sends the close packet, then closes the websocket, which is what the client
        /// acts on.
        #[test]
        fn forcefully_closes_the_session() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send("1");
                peer.close();
            });

            let mut client = joined(address, "/", None);
            assert_disconnect(next(&mut client), DisconnectReason::TransportClose);
            server.join().unwrap();
        }

        // This client only.

        /// engine.io-client ignores it over websocket.
        #[test]
        fn ignores_a_close_packet_until_the_websocket_closes() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send("1");
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            finish(client, server);
        }

        /// No close frame, as when the server dies. Still a transport close to the reference client.
        #[test]
        fn closes_the_session_when_the_connection_drops() {
            let (address, server) = serve(|mut listener| listener.accept().accept("40"));

            let mut client = joined(address, "/", None);
            server.join().unwrap();
            assert_disconnect(next(&mut client), DisconnectReason::TransportClose);
        }

        #[test]
        fn reads_what_the_server_sent_before_a_failed_write() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"42["last"]"#);
            });

            let mut client = client(address, "/", None, once());
            // A frame per step, so "last" is still unread.
            assert_connect(next(&mut client), address);
            server.join().unwrap();
            // The first write draws a reset, which fails a later one.
            for _ in 0..20 {
                client.emit(Frame::event("/", "x", vec![]).unwrap());
                std::thread::sleep(Duration::from_millis(20));
            }
            // Windows discards unread data on a reset.
            #[cfg(not(windows))]
            assert_message(next(&mut client), "last", vec![]);
            loop {
                match next(&mut client) {
                    Event::Message { .. } => {}
                    event => break assert_disconnect(event, DisconnectReason::TransportClose),
                }
            }
        }
    }
}

mod socket_io {
    use super::*;

    mod connect {
        use super::*;

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L393>
        #[test]
        fn allows_connection_to_the_main_namespace() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"42["auth",{}]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "auth", args(json!([{}])));
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L416>
        #[test]
        fn allows_connection_to_the_main_namespace_with_a_payload() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept(r#"40{"token":"123"}"#);
                peer.send(r#"42["auth",{"token":"123"}]"#);
            });

            let mut client = joined(address, "/", Some(json!({ "token": "123" })));
            assert_message(next(&mut client), "auth", args(json!([{ "token": "123" }])));
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L439>
        #[test]
        fn allows_connection_to_a_custom_namespace() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40/custom,");
                peer.send(r#"42/custom,["auth",{}]"#);
            });

            let mut client = joined(address, NS, None);
            assert_message(next(&mut client), "auth", args(json!([{}])));
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L462>
        #[test]
        fn allows_connection_to_a_custom_namespace_with_a_payload() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept(r#"40/custom,{"token":"abc"}"#);
                peer.send(r#"42/custom,["auth",{"token":"abc"}]"#);
            });

            let mut client = joined(address, NS, Some(json!({ "token": "abc" })));
            assert_message(next(&mut client), "auth", args(json!([{ "token": "abc" }])));
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L485>
        #[test]
        fn disallows_connection_to_an_unknown_namespace() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.send(support::OPEN);
                assert_eq!(peer.recv(), "40/random,");
                peer.send(r#"44/random,{"message":"Invalid namespace"}"#);
                peer.wait_closed();
            });

            let mut client = client(address, "/random", None, once());
            match next(&mut client) {
                Event::ConnectError(ConnectError::Refused { message, data }) => {
                    assert_eq!(message, "Invalid namespace");
                    assert_eq!(data, None);
                }
                other => panic!("{other:?}"),
            }
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L499>
        ///
        /// Turned around: the server answers the client's `CONNECT` with the suite's invalid packet.
        #[test]
        fn disallows_connection_with_an_invalid_handshake() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.send(support::OPEN);
                peer.recv();
                peer.send("4abc");
                peer.wait_closed();
            });

            let mut client = client(address, NS, None, once());
            assert_parse_error(next(&mut client));
            drop(client);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L512>
        ///
        /// Turned around: the client waits for the server to close it, as the reference client does.
        #[test]
        fn closes_the_connection_if_no_handshake_is_received() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.send(support::OPEN);
                peer.recv();
                peer.close();
            });

            let mut client = client(address, NS, None, once());
            assert!(matches!(
                next(&mut client),
                Event::Disconnect(DisconnectReason::TransportClose)
            ));
            server.join().unwrap();
        }
    }

    mod disconnect {
        use super::*;

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L522>
        #[test]
        fn disconnects_from_the_main_namespace() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                assert_eq!(peer.recv(), "41");
                peer.wait_closed();
            });

            joined(address, "/", None).close();
            server.join().unwrap();
        }

        // This client only.

        /// The reference `Socket.onpacket` ignores namespaces other than its own.
        #[test]
        fn ignores_namespaces_it_has_not_joined() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.send(support::OPEN);
                peer.recv();
                peer.send(r#"44{"message":"Not authorized"}"#);
                peer.send(r#"40{"sid":"oSO0OpakMV_3jnilAAAA"}"#);
                peer.send("41");
                peer.send(r#"42["message-back","message to main namespace"]"#);
                peer.send(r#"40/custom,{"sid":"wZX3oN0bSVIhsaknAAAI"}"#);
                peer.send(r#"42/custom,["after"]"#);
            });

            let mut client = client(address, NS, None, once());
            assert_connect(next(&mut client), address);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }
    }

    mod message {
        use super::*;

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L552>
        #[test]
        fn sends_a_plain_text_packet() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                assert_eq!(peer.recv(), r#"42["message",1,"2",{"3":[true]}]"#);
                peer.send(r#"42["message-back",1,"2",{"3":[true]}]"#);
            });

            let mut client = joined(address, "/", None);
            let message = args(json!([1, "2", { "3": [true] }]));
            client.emit(Frame::event("/", "message", message.clone()).unwrap());
            assert_message(next(&mut client), "message-back", message);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L562>
        ///
        /// Turned around: the server sends the suite's reply, which the client drops, since it sends no binary.
        #[test]
        fn sends_a_packet_with_binary_attachments() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"452-["message-back",{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#);
                peer.send_binary(&[1, 2, 3]);
                peer.send_binary(&[4, 5, 6]);
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L582>
        ///
        /// Turned around: the server asks for the ack and the client answers.
        #[test]
        fn sends_a_plain_text_packet_with_an_ack() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"42456["message-with-ack",1,"2",{"3":[false]}]"#);
                assert_eq!(peer.recv(), r#"43456[1,"2",{"3":[false]}]"#);
            });

            let mut client = joined(address, "/", None);
            let Event::Message {
                args,
                ack: Some(responder),
                ..
            } = next(&mut client)
            else {
                panic!("expected an event awaiting an ack");
            };
            client.ack(responder, args);
            finish(client, server);
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L592>
        ///
        /// Turned around: the server asks for the ack, and the client drops the binary event.
        #[test]
        fn sends_a_packet_with_binary_attachments_and_an_ack() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(
                    r#"452-789["message-with-ack",{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#,
                );
                peer.send_binary(&[1, 2, 3]);
                peer.send_binary(&[4, 5, 6]);
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L612>
        #[test]
        fn closes_the_connection_upon_invalid_format_unknown_packet_type() {
            assert_rejects("4abc");
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L620>
        #[test]
        fn closes_the_connection_upon_invalid_format_invalid_payload_format() {
            assert_rejects("42{}");
        }

        /// <https://github.com/socketio/socket.io-protocol/blob/4f633dd/test-suite/test-suite.js#L628>
        #[test]
        fn closes_the_connection_upon_invalid_format_invalid_ack_id() {
            assert_rejects(r#"42abc["message-with-ack",1,"2",{"3":[false]}]"#);
        }

        // This client only.

        #[test]
        fn closes_the_connection_upon_invalid_format_in_a_namespace_not_joined() {
            assert_rejects("42/other,{}");
        }

        /// The client never asks for one.
        #[test]
        fn ignores_acks() {
            let (address, server) = serve(|mut listener| {
                let mut peer = listener.accept();
                peer.accept("40");
                peer.send(r#"430[1]"#);
                peer.send(r#"42["after"]"#);
            });

            let mut client = joined(address, "/", None);
            assert_message(next(&mut client), "after", vec![]);
            server.join().unwrap();
        }

        #[test]
        fn refuses_reserved_event_names() {
            assert!(Frame::event("/", "disconnect", vec![]).is_err());
        }
    }
}
