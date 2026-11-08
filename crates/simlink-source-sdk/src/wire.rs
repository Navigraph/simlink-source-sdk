//! Socket.IO v5 packets over Engine.IO v4, websocket transport, text frames only.

use std::time::Duration;

use serde_json::Value;

/// Engine.IO heartbeat reply.
pub const PONG: &str = "3";

/// A decoded frame addressed to this client.
#[derive(Debug, PartialEq)]
pub enum Inbound {
    Open {
        ping_interval: Duration,
        ping_timeout: Duration,
    },
    Ping,
    Close,
    Connected,
    ConnectError {
        message: String,
        data: Option<Value>,
    },
    Disconnected,
    Event {
        name: String,
        args: Vec<Value>,
        ack: Option<u64>,
    },
    /// Valid, but nothing this client acts on, such as another namespace's traffic.
    Ignored,
}

#[derive(Debug, PartialEq)]
pub struct MalformedFrame(pub String);

impl std::fmt::Display for MalformedFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed frame: {}", self.0)
    }
}

pub fn connect(namespace: &str, auth: &Value) -> String {
    format!("40{namespace},{auth}")
}

pub fn disconnect(namespace: &str) -> String {
    format!("41{namespace},")
}

pub fn event(namespace: &str, name: &str, payload: &Value) -> String {
    format!(
        "42{namespace},{}",
        Value::Array(vec![Value::from(name), payload.clone()])
    )
}

pub fn ack(namespace: &str, id: u64, value: &Value) -> String {
    format!("43{namespace},{id}{}", Value::Array(vec![value.clone()]))
}

pub fn decode(frame: &str, namespace: &str) -> Result<Inbound, MalformedFrame> {
    let malformed = || MalformedFrame(frame.chars().take(80).collect());
    let mut chars = frame.chars();

    match chars.next() {
        Some('0') => {
            let open: Value = serde_json::from_str(chars.as_str()).map_err(|_| malformed())?;
            let millis = |key: &str| {
                open.get(key)
                    .and_then(Value::as_u64)
                    .map(Duration::from_millis)
            };

            Ok(Inbound::Open {
                ping_interval: millis("pingInterval").ok_or_else(malformed)?,
                ping_timeout: millis("pingTimeout").ok_or_else(malformed)?,
            })
        }
        Some('1') => Ok(Inbound::Close),
        Some('2') => Ok(Inbound::Ping),
        Some('4') => decode_packet(chars.as_str(), namespace).ok_or_else(malformed),
        Some('3' | '5' | '6') => Ok(Inbound::Ignored),
        _ => Err(malformed()),
    }
}

fn decode_packet(packet: &str, namespace: &str) -> Option<Inbound> {
    let (kind, rest) = packet.split_at_checked(1)?;

    let (packet_namespace, rest) = match rest.strip_prefix('/') {
        Some(_) => rest.split_once(',').unwrap_or((rest, "")),
        None => ("/", rest),
    };

    if packet_namespace != namespace {
        return Some(Inbound::Ignored);
    }

    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (ack, body) = rest.split_at(digits);
    let ack = if ack.is_empty() {
        None
    } else {
        Some(ack.parse().ok()?)
    };
    let body = || serde_json::from_str::<Value>(body).ok();

    match kind {
        "0" => Some(Inbound::Connected),
        "1" => Some(Inbound::Disconnected),
        "2" => {
            let Value::Array(mut args) = body()? else {
                return None;
            };
            if args.is_empty() {
                return None;
            }
            let Value::String(name) = args.remove(0) else {
                return None;
            };

            Some(Inbound::Event { name, args, ack })
        }
        "4" => {
            let error = body()?;

            Some(Inbound::ConnectError {
                message: error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                data: error.get("data").cloned(),
            })
        }
        // Binary events and acks: Simlink sends none, and dropping one silently would lose a command.
        "5" | "6" => None,
        _ => Some(Inbound::Ignored),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NS: &str = "/v1/source";

    #[test]
    fn decodes_engine_open() {
        let frame = r#"0{"sid":"a","upgrades":[],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}"#;

        assert_eq!(
            decode(frame, NS),
            Ok(Inbound::Open {
                ping_interval: Duration::from_secs(25),
                ping_timeout: Duration::from_secs(20)
            })
        );
    }

    #[test]
    fn decodes_namespace_connect_outcomes() {
        assert_eq!(
            decode(r#"40/v1/source,{"sid":"x"}"#, NS),
            Ok(Inbound::Connected)
        );
        assert_eq!(
            decode(
                r#"44/v1/source,{"message":"taken","data":{"code":"HANDSHAKE_SOURCE_ID_IN_USE"}}"#,
                NS
            ),
            Ok(Inbound::ConnectError {
                message: "taken".into(),
                data: Some(json!({ "code": "HANDSHAKE_SOURCE_ID_IN_USE" }))
            })
        );
        assert_eq!(decode("41/v1/source,", NS), Ok(Inbound::Disconnected));
    }

    #[test]
    fn decodes_events_with_and_without_acks() {
        assert_eq!(
            decode(r#"42/v1/source,13["atis.get",{"icao":"ESSA"}]"#, NS),
            Ok(Inbound::Event {
                name: "atis.get".into(),
                args: vec![json!({ "icao": "ESSA" })],
                ack: Some(13)
            })
        );
        assert_eq!(
            decode(
                r#"42/v1/source,["simlink:error",{"code":"INVALID_DATA"}]"#,
                NS
            ),
            Ok(Inbound::Event {
                name: "simlink:error".into(),
                args: vec![json!({ "code": "INVALID_DATA" })],
                ack: None
            })
        );
    }

    #[test]
    fn ignores_other_namespaces() {
        assert_eq!(
            decode(r#"42/v1/consumer,["x",1]"#, NS),
            Ok(Inbound::Ignored)
        );
        assert_eq!(decode(r#"42["x",1]"#, NS), Ok(Inbound::Ignored));
    }

    #[test]
    fn rejects_malformed_frames() {
        assert!(decode("", NS).is_err());
        assert!(decode("42/v1/source,[1]", NS).is_err());
        assert!(decode("42/v1/source,12x", NS).is_err());
    }

    #[test]
    fn encodes_outbound_packets() {
        assert_eq!(
            connect(NS, &json!({ "id": "a" })),
            r#"40/v1/source,{"id":"a"}"#
        );
        assert_eq!(
            event(NS, "telemetry.traffic", &json!([])),
            r#"42/v1/source,["telemetry.traffic",[]]"#
        );
        assert_eq!(
            ack(NS, 7, &json!({ "ok": true, "value": [] })),
            r#"43/v1/source,7[{"ok":true,"value":[]}]"#
        );
        assert_eq!(disconnect(NS), "41/v1/source,");
    }
}
