//! Socket.IO v5 packets in Engine.IO v4 websocket frames.
//!
//! Decoding covers every packet a server can send, binary included, so any frame is either understood or rejected.
//! Validation follows the spec, then the reference parser where the spec is silent. Encoding covers only what this
//! client sends.
//!
//! - <https://github.com/socketio/socket.io-protocol#packet-encoding>
//! - <https://github.com/socketio/socket.io/blob/main/packages/socket.io-parser/lib/index.ts>

use std::borrow::Cow;
use std::time::Duration;

use serde_json::{Map, Value};

/// Engine.IO heartbeat reply.
pub const PONG: &str = "3";

/// The reference parser's default `maxAttachments`.
const MAX_ATTACHMENTS: usize = 10;

/// Event names the reference parser rejects.
const RESERVED_EVENTS: [&str; 6] = [
    "connect",
    "connect_error",
    "disconnect",
    "disconnecting",
    "newListener",
    "removeListener",
];

/// A frame that breaks the protocol.
#[derive(Debug, PartialEq)]
pub struct Malformed {
    reason: &'static str,
    frame: String,
}

impl Malformed {
    fn new(reason: &'static str, frame: &str) -> Malformed {
        Malformed {
            reason,
            frame: frame.chars().take(80).collect(),
        }
    }
}

impl std::fmt::Display for Malformed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed frame ({}): {}", self.reason, self.frame)
    }
}

/// An Engine.IO packet from a text frame.
/// <https://github.com/socketio/engine.io-protocol#protocol>
#[derive(Debug, PartialEq)]
pub enum Engine<'a> {
    Open {
        ping_interval: Duration,
        ping_timeout: Duration,
    },
    Ping,
    /// A Socket.IO packet.
    Message(&'a str),
    /// A base64 binary message, which engine.io-parser takes over websocket too.
    Binary,
    /// Close, pong, upgrade and noop. The reference client ignores these over websocket, where a closing server
    /// closes the websocket.
    Ignored,
}

pub fn decode_engine(frame: &str) -> Result<Engine<'_>, Malformed> {
    let mut chars = frame.chars();

    match chars.next() {
        // https://github.com/socketio/engine.io-protocol#handshake
        Some('0') => {
            let open: Value = serde_json::from_str(chars.as_str())
                .map_err(|_| Malformed::new("invalid open packet", frame))?;
            let millis = |key: &str| {
                open.get(key)
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis)
                    .ok_or_else(|| Malformed::new("invalid open packet", frame))
            };

            Ok(Engine::Open {
                ping_interval: millis("pingInterval")?,
                ping_timeout: millis("pingTimeout")?,
            })
        }
        Some('2') => Ok(Engine::Ping),
        Some('4') => Ok(Engine::Message(chars.as_str())),
        Some('b') => Ok(Engine::Binary),
        Some('1' | '3' | '5' | '6') => Ok(Engine::Ignored),
        _ => Err(Malformed::new("unknown Engine.IO packet type", frame)),
    }
}

/// A complete Socket.IO packet.
#[derive(Debug, PartialEq)]
pub enum Inbound {
    Connected {
        namespace: String,
        /// Whether the answer has the required session id, which v2 servers omit.
        with_sid: bool,
    },
    Disconnected {
        namespace: String,
    },
    ConnectError {
        namespace: String,
        message: String,
        data: Option<Value>,
    },
    Event {
        namespace: String,
        name: String,
        args: Vec<Value>,
        id: Option<u64>,
    },
    /// A valid ack or binary packet, neither of which this client uses.
    Ignored(&'static str),
}

/// Turns frames into packets, consuming a binary packet's attachments before handing it over.
#[derive(Default)]
pub struct Decoder {
    /// A binary packet and how many attachments are still to come.
    pending: Option<(Inbound, usize)>,
}

impl Decoder {
    /// Takes a Socket.IO packet; `None` while it waits for attachments.
    pub fn text(&mut self, packet: &str) -> Result<Option<Inbound>, Malformed> {
        if self.pending.is_some() {
            return Err(Malformed::new("text while awaiting attachments", packet));
        }

        match decode(packet)? {
            (inbound, 0) => Ok(Some(inbound)),
            (inbound, attachments) => {
                self.pending = Some((inbound, attachments));
                Ok(None)
            }
        }
    }

    /// Takes a binary attachment; the packet once it's complete.
    pub fn binary(&mut self) -> Result<Option<Inbound>, Malformed> {
        match self.pending.take() {
            None => Err(Malformed::new("binary outside a packet", "<binary>")),
            Some((inbound, 1)) => Ok(Some(inbound)),
            Some((inbound, left)) => {
                self.pending = Some((inbound, left - 1));
                Ok(None)
            }
        }
    }
}

/// `<type>[<attachments>-][<namespace>,][<id>][<JSON payload>]`, with the number of attachments to follow:
/// <https://github.com/socketio/socket.io-protocol#format>.
fn decode(text: &str) -> Result<(Inbound, usize), Malformed> {
    let malformed = |reason| Malformed::new(reason, text);

    let (kind, rest) = text
        .split_at_checked(1)
        .ok_or_else(|| malformed("empty packet"))?;
    let binary = matches!(kind, "5" | "6");

    let (attachments, rest) = if binary {
        let (count, rest) = rest
            .split_once('-')
            .ok_or_else(|| malformed("invalid attachments"))?;
        // The reference parser requires `1..=MAX_ATTACHMENTS`.
        let count = Some(count)
            .filter(|count| !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|count| count.parse().ok())
            .filter(|count| (1..=MAX_ATTACHMENTS).contains(count))
            .ok_or_else(|| malformed("invalid attachments"))?;
        (count, rest)
    } else {
        (0, rest)
    };

    // The main namespace is implied.
    let (namespace, rest) = match rest.strip_prefix('/') {
        Some(_) => rest.split_once(',').unwrap_or((rest, "")),
        None => ("/", rest),
    };
    let namespace = namespace.to_owned();

    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (id, payload) = rest.split_at(end);
    let id = match id {
        "" => None,
        id => Some(id.parse().map_err(|_| malformed("invalid ack id"))?),
    };
    let payload = match payload {
        "" => None,
        payload => Some(
            serde_json::from_str(&replace_lone_surrogates(payload))
                .map_err(|_| malformed("invalid payload"))?,
        ),
    };

    let inbound = match (kind, payload, id) {
        // https://github.com/socketio/socket.io-protocol#connection-to-a-namespace
        // The reference client checks `packet.data && packet.data.sid`.
        ("0", None, _) => Inbound::Connected {
            namespace,
            with_sid: false,
        },
        ("0", Some(Value::Object(payload)), _) => Inbound::Connected {
            namespace,
            with_sid: payload.get("sid").is_some_and(truthy),
        },
        // https://github.com/socketio/socket.io-protocol#disconnection-from-a-namespace
        ("1", None, _) => Inbound::Disconnected { namespace },
        // The reference parser allows a string, but the client reads `message`, which a string lacks.
        ("4", None | Some(Value::String(_)), _) => Inbound::ConnectError {
            namespace,
            message: String::new(),
            data: None,
        },
        ("4", Some(Value::Object(mut payload)), _) => Inbound::ConnectError {
            namespace,
            message: payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            data: payload.remove("data"),
        },
        // https://github.com/socketio/socket.io-protocol#sending-and-receiving-data: the payload "MUST be a
        // non-empty array". The reference parser also takes a number as the name.
        ("2" | "5", Some(Value::Array(mut args)), id) if !args.is_empty() => {
            let name = match args.remove(0) {
                Value::String(name) if !RESERVED_EVENTS.contains(&name.as_str()) => name,
                Value::Number(name) => name.to_string(),
                _ => return Err(malformed("invalid event name")),
            };
            if binary {
                check_placeholders(&args, attachments)
                    .ok_or_else(|| malformed("invalid attachments"))?;
                Inbound::Ignored("binary event")
            } else {
                Inbound::Event {
                    namespace,
                    name,
                    args,
                    id,
                }
            }
        }
        // https://github.com/socketio/socket.io-protocol#acknowledgement: the payload "MUST be an array".
        ("3" | "6", Some(Value::Array(args)), Some(_)) => {
            if binary {
                check_placeholders(&args, attachments)
                    .ok_or_else(|| malformed("invalid attachments"))?;
            }
            Inbound::Ignored("ack")
        }
        ("0" | "1" | "2" | "3" | "4" | "5" | "6", _, _) => {
            return Err(malformed("invalid payload"));
        }
        _ => return Err(malformed("unknown packet type")),
    };

    Ok((inbound, attachments))
}

/// Replaces unpaired UTF-16 surrogate escapes with U+FFFD. Node's `JSON.stringify` writes them for strings cut
/// mid-character and `JSON.parse` accepts them, but serde_json doesn't, and a Rust string can't hold one.
fn replace_lone_surrogates(json: &str) -> Cow<'_, str> {
    if !json.contains("\\u") {
        return Cow::Borrowed(json);
    }

    let mut replaced = String::with_capacity(json.len());
    let mut rest = json;
    while let Some(start) = rest.find('\\') {
        replaced.push_str(&rest[..start]);
        rest = &rest[start..];
        let length = match surrogate(rest) {
            // A high surrogate paired with a low one.
            Some(0xD800..0xDC00)
                if surrogate(&rest[6..]).is_some_and(|low| (0xDC00..0xE000).contains(&low)) =>
            {
                12
            }
            Some(_) => {
                replaced.push_str("\\ufffd");
                rest = &rest[6..];
                continue;
            }
            // Any other escape, kept whole so an escaped backslash can't start one.
            None => 1 + rest[1..].chars().next().map_or(0, char::len_utf8),
        };
        replaced.push_str(&rest[..length]);
        rest = &rest[length..];
    }
    replaced.push_str(rest);
    Cow::Owned(replaced)
}

/// The UTF-16 surrogate that `text` starts with as a `\uXXXX` escape.
fn surrogate(text: &str) -> Option<u16> {
    let hex = text.strip_prefix("\\u")?.get(..4)?;
    let unit = u16::from_str_radix(hex, 16).ok()?;
    (0xD800..0xE000).contains(&unit).then_some(unit)
}

/// Whether JavaScript would take `value` as true.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Whether every binary placeholder refers to one of `attachments`.
fn check_placeholders<'a>(
    values: impl IntoIterator<Item = &'a Value>,
    attachments: usize,
) -> Option<()> {
    for value in values {
        match value {
            Value::Object(entries) if entries.get("_placeholder") == Some(&Value::Bool(true)) => {
                let num = entries.get("num")?.as_u64()?;
                (num < attachments as u64).then_some(())?;
            }
            Value::Object(entries) => check_placeholders(entries.values(), attachments)?,
            Value::Array(values) => check_placeholders(values, attachments)?,
            _ => {}
        }
    }
    Some(())
}

pub fn connect(namespace: &str, auth: Option<&Map<String, Value>>) -> String {
    let auth = auth.map(|auth| Value::Object(auth.clone()).to_string());
    format!("40{}{}", prefix(namespace), auth.unwrap_or_default())
}

pub fn disconnect(namespace: &str) -> String {
    format!("41{}", prefix(namespace))
}

/// `None` for a reserved name.
pub fn event(namespace: &str, name: &str, args: Vec<Value>) -> Option<String> {
    if RESERVED_EVENTS.contains(&name) {
        return None;
    }

    let mut payload = vec![Value::String(name.to_owned())];
    payload.extend(args);
    Some(format!("42{}{}", prefix(namespace), Value::Array(payload)))
}

pub fn ack(namespace: &str, id: u64, args: Vec<Value>) -> String {
    format!("43{}{id}{}", prefix(namespace), Value::Array(args))
}

fn prefix(namespace: &str) -> String {
    if namespace == "/" {
        String::new()
    } else {
        format!("{namespace},")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn decode(text: &str) -> Result<Option<Inbound>, Malformed> {
        Decoder::default().text(text)
    }

    #[test]
    fn decodes_engine_packets() {
        // https://github.com/socketio/engine.io-protocol#handshake
        let frame = r#"0{"sid":"lv_VI97HAXpY6yYWAAAC","upgrades":["websocket"],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}"#;

        assert_eq!(
            decode_engine(frame),
            Ok(Engine::Open {
                ping_interval: Duration::from_secs(25),
                ping_timeout: Duration::from_secs(20)
            })
        );
        assert_eq!(decode_engine("2"), Ok(Engine::Ping));
        assert_eq!(decode_engine("1"), Ok(Engine::Ignored));
        assert_eq!(decode_engine("bAQID"), Ok(Engine::Binary));
        // https://github.com/socketio/engine.io-protocol#websocket-1
        assert_eq!(decode_engine("4hello"), Ok(Engine::Message("hello")));
        assert!(decode_engine("9").is_err());
        assert!(decode_engine("").is_err());
    }

    /// <https://github.com/socketio/socket.io-protocol#examples>
    #[test]
    fn decodes_the_spec_examples() {
        assert_eq!(
            decode(r#"0/admin,{"sid":"oSO0OpakMV_3jnilAAAA"}"#),
            Ok(Some(Inbound::Connected {
                namespace: "/admin".into(),
                with_sid: true
            }))
        );
        for without in ["0", r#"0{}"#, r#"0{"sid":""}"#] {
            assert_eq!(
                decode(without),
                Ok(Some(Inbound::Connected {
                    namespace: "/".into(),
                    with_sid: false
                }))
            );
        }
        assert_eq!(
            decode(r#"4{"message":"Not authorized"}"#),
            Ok(Some(Inbound::ConnectError {
                namespace: "/".into(),
                message: "Not authorized".into(),
                data: None
            }))
        );
        assert_eq!(
            decode(r#"2/admin,["bar"]"#),
            Ok(Some(Inbound::Event {
                namespace: "/admin".into(),
                name: "bar".into(),
                args: vec![],
                id: None
            }))
        );
        assert_eq!(
            decode(r#"212["foo"]"#),
            Ok(Some(Inbound::Event {
                namespace: "/".into(),
                name: "foo".into(),
                args: vec![],
                id: Some(12)
            }))
        );
        assert_eq!(
            decode(r#"3/admin,13["bar"]"#),
            Ok(Some(Inbound::Ignored("ack")))
        );
        // Only binary acks have placeholders to check.
        assert_eq!(
            decode(r#"30[{"_placeholder":true,"num":0}]"#),
            Ok(Some(Inbound::Ignored("ack")))
        );
        assert_eq!(
            decode("1/admin,"),
            Ok(Some(Inbound::Disconnected {
                namespace: "/admin".into()
            }))
        );
    }

    /// <https://github.com/socketio/socket.io-protocol#examples>
    #[test]
    fn consumes_binary_packets() {
        let mut decoder = Decoder::default();

        assert_eq!(
            decoder.text(
                r#"52-/admin,["baz",{"_placeholder":true,"num":0},{"_placeholder":true,"num":1}]"#
            ),
            Ok(None)
        );
        assert_eq!(decoder.binary(), Ok(None));
        assert_eq!(decoder.binary(), Ok(Some(Inbound::Ignored("binary event"))));
        assert_eq!(
            decoder.text(r#"61-15["bar",{"_placeholder":true,"num":0}]"#),
            Ok(None)
        );
        assert_eq!(decoder.binary(), Ok(Some(Inbound::Ignored("ack"))));
        // The reference parser's limit.
        assert_eq!(decoder.text(r#"510-["foo"]"#), Ok(None));
    }

    #[test]
    fn rejects_out_of_order_attachments() {
        let mut decoder = Decoder::default();
        assert!(decoder.binary().is_err());

        decoder
            .text(r#"51-["baz",{"_placeholder":true,"num":0}]"#)
            .unwrap();
        assert!(decoder.text(r#"2["foo"]"#).is_err());
    }

    #[test]
    fn rejects_what_the_spec_or_the_reference_parser_rejects() {
        for packet in [
            "",
            "abc",
            "7",
            r#"0[]"#,
            r#"1{}"#,
            r#"2"#,
            r#"2{}"#,
            r#"2[]"#,
            r#"2[{}]"#,
            r#"2["connect"]"#,
            r#"2abc["foo"]"#,
            r#"3[]"#,
            r#"4[]"#,
            r#"5["foo"]"#,
            r#"5x-["foo"]"#,
            r#"50-["foo"]"#,
            r#"511-["foo"]"#,
            r#"51-["foo",{"_placeholder":true,"num":1}]"#,
            r#"51-["foo",{"_placeholder":true}]"#,
        ] {
            assert!(decode(packet).is_err(), "{packet}");
        }
    }

    #[test]
    fn replaces_lone_surrogates_as_json_parse_takes_them() {
        assert_eq!(
            decode(r#"2["a","\ud800","x\udc00y","\ud83d\ude00","\\ud800"]"#),
            Ok(Some(Inbound::Event {
                namespace: "/".into(),
                name: "a".into(),
                args: vec![
                    json!("\u{fffd}"),
                    json!("x\u{fffd}y"),
                    json!("😀"),
                    json!("\\ud800")
                ],
                id: None
            }))
        );
    }

    #[test]
    fn takes_a_connect_error_message_from_an_object_only() {
        assert_eq!(
            decode(r#"4"Not authorized""#),
            Ok(Some(Inbound::ConnectError {
                namespace: "/".into(),
                message: String::new(),
                data: None
            }))
        );
    }

    #[test]
    fn accepts_numeric_event_names() {
        assert_eq!(
            decode(r#"2[1,"x"]"#),
            Ok(Some(Inbound::Event {
                namespace: "/".into(),
                name: "1".into(),
                args: vec![json!("x")],
                id: None
            }))
        );
    }

    /// <https://github.com/socketio/socket.io-protocol#examples>
    #[test]
    fn encodes_the_spec_examples() {
        assert_eq!(connect("/", None), "40");
        assert_eq!(
            connect("/admin", json!({ "token": "123" }).as_object()),
            r#"40/admin,{"token":"123"}"#
        );
        assert_eq!(disconnect("/"), "41");
        assert_eq!(disconnect("/admin"), "41/admin,");
        assert_eq!(event("/", "foo", vec![]).unwrap(), r#"42["foo"]"#);
        assert_eq!(
            event("/admin", "bar", vec![json!(1)]).unwrap(),
            r#"42/admin,["bar",1]"#
        );
        assert_eq!(
            ack("/admin", 13, vec![json!("bar")]),
            r#"43/admin,13["bar"]"#
        );
        assert_eq!(event("/", "disconnect", vec![]), None);
    }
}
