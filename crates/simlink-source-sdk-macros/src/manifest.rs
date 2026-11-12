//! Reads the manifest into the model the generators work from, failing on anything the SDK doesn't handle.

use serde_json::{Map, Value};

use crate::names::pascal;

/// A schema typify turns into a Rust type, with the name it gets if it's an object.
pub struct Schema {
    pub name: Option<String>,
    pub schema: Value,
}

pub struct Channel {
    pub name: String,
    pub doc: String,
    pub deprecated: Option<String>,
}

pub struct Event {
    pub channel: Channel,
    pub max_rate_hz: Option<f64>,
    pub payload: Schema,
}

pub struct Command {
    pub channel: Channel,
    pub timeout_ms: u64,
    pub input: Schema,
    pub output: Schema,
    pub error: Schema,
}

/// One of the codes Simlink refuses a handshake or rejects a frame with.
pub struct Code {
    pub code: String,
    pub doc: String,
    pub variant: String,
    pub fields: Vec<Field>,
}

/// A field of a code's metadata.
pub struct Field {
    pub key: String,
    pub ty: FieldType,
    pub required: bool,
}

/// The JSON types every binding has.
pub enum FieldType {
    String,
    Integer,
    Number,
    Boolean,
    Array(Box<FieldType>),
}

/// What the SDK connects with.
pub struct Connection {
    pub protocol_version: u32,
    pub default_port: u16,
    pub port_probe_limit: u16,
    pub max_payload_bytes: usize,
    pub namespace: String,
    pub mdns_service_type: String,
    pub error_channel: String,
    pub socket_io_path: String,
}

pub struct Manifest {
    /// `SourceOptions`: the descriptor without the capabilities, which the SDK declares.
    pub descriptor: Schema,
    pub events: Vec<Event>,
    pub commands: Vec<Command>,
    pub handshake_errors: Vec<Code>,
    pub rejection_codes: Vec<Code>,
    pub connection: Connection,
}

impl Manifest {
    /// Every event, then every command.
    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        let events = self.events.iter().map(|event| &event.channel);
        events.chain(self.commands.iter().map(|command| &command.channel))
    }
}

pub fn read(text: &str) -> Result<Manifest, String> {
    let manifest: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    check_keys(&manifest)?;

    let handshake_errors = manifest["handshakeErrors"]["oneOf"]
        .as_array()
        .ok_or("handshakeErrors.oneOf is missing")?;

    Ok(Manifest {
        descriptor: descriptor(&manifest["descriptor"])?,
        events: object(&manifest["events"], "events")?
            .iter()
            .map(|(name, entry)| event(name, entry))
            .collect::<Result<_, _>>()?,
        commands: object(&manifest["commands"], "commands")?
            .iter()
            .map(|(name, entry)| command(name, entry))
            .collect::<Result<_, _>>()?,
        handshake_errors: handshake_errors
            .iter()
            .map(handshake_error)
            .collect::<Result<_, _>>()?,
        rejection_codes: rejection_codes(&manifest["errorFrame"])?,
        connection: connection(&manifest["connection"])?,
    })
}

fn descriptor(descriptor: &Value) -> Result<Schema, String> {
    let mut schema = descriptor.clone();
    schema["properties"]
        .as_object_mut()
        .ok_or("descriptor.properties is missing")?
        .remove("capabilities")
        .ok_or("the SDK declares descriptor.capabilities, but the protocol has none")?;
    if let Some(required) = schema["required"].as_array_mut() {
        required.retain(|name| name != "capabilities");
    }
    schema["description"] =
        "Source identity presented to Simlink. The SDK declares the capabilities.".into();

    Ok(Schema {
        name: Some("SourceOptions".into()),
        schema,
    })
}

fn event(name: &str, entry: &Value) -> Result<Event, String> {
    Ok(Event {
        channel: channel(name, entry)?,
        max_rate_hz: entry["maxRateHz"].as_f64(),
        payload: named_if_object(&entry["payload"], pascal(name)),
    })
}

fn command(name: &str, entry: &Value) -> Result<Command, String> {
    let base = pascal(name);
    Ok(Command {
        channel: channel(name, entry)?,
        timeout_ms: field(entry, name, "timeoutMs", Value::as_u64)?,
        input: Schema {
            name: Some(format!("{base}Input")),
            schema: entry["input"].clone(),
        },
        output: named_if_object(&entry["output"], format!("{base}Output")),
        error: Schema {
            name: Some(format!("{base}Error")),
            schema: entry["errors"].clone(),
        },
    })
}

/// Objects get a channel-derived name; arrays and maps stay `Vec<T>` and `HashMap<String, T>`.
fn named_if_object(schema: &Value, name: String) -> Schema {
    Schema {
        name: schema.get("properties").map(|_| name),
        schema: schema.clone(),
    }
}

/// What the generated API needs from a channel entry. The SDK only emits and answers on behalf of a source; other
/// channels would point the wrong way.
fn channel(name: &str, entry: &Value) -> Result<Channel, String> {
    match entry["origin"].as_str() {
        Some("source") => {}
        origin => {
            return Err(format!(
                "{name} has origin {origin:?}; the SDK only handles source channels"
            ));
        }
    }
    let deprecated = match &entry["deprecated"] {
        Value::String(reason) => Some(reason.clone()),
        Value::Bool(true) => Some(String::new()),
        _ => None,
    };

    Ok(Channel {
        name: name.to_owned(),
        doc: entry["description"].as_str().unwrap_or_default().to_owned(),
        deprecated,
    })
}

/// A `handshakeErrors` entry: its code, and metadata fields of the types the SDK maps.
fn handshake_error(error: &Value) -> Result<Code, String> {
    let code = field(
        error,
        "handshakeErrors",
        "properties/code/const",
        Value::as_str,
    )?;
    let doc = error["properties"]["code"]["description"]
        .as_str()
        .unwrap_or_default();
    let metadata = &error["properties"]["metadata"];
    let required = metadata["required"].as_array().cloned().unwrap_or_default();

    let fields = metadata["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, schema)| {
            Ok(Field {
                key: key.clone(),
                ty: field_type(schema).ok_or(format!(
                    "handshakeErrors {code} has metadata.{key} of a type the SDK doesn't map yet"
                ))?,
                required: required.iter().any(|name| name == key),
            })
        })
        .collect::<Result<_, String>>()?;

    new_code(code, "HANDSHAKE_", doc, fields)
}

fn field_type(schema: &Value) -> Option<FieldType> {
    Some(match schema["type"].as_str()? {
        "string" => FieldType::String,
        "integer" => FieldType::Integer,
        "number" => FieldType::Number,
        "boolean" => FieldType::Boolean,
        "array" => FieldType::Array(Box::new(field_type(&schema["items"])?)),
        _ => return None,
    })
}

/// The error frame's codes. Also checks the frame fields the rejection listener reads by name.
fn rejection_codes(frame: &Value) -> Result<Vec<Code>, String> {
    for key in [
        "event",
        "message",
        "issues/items/properties/path",
        "issues/items/properties/message",
        "issues/items/properties/code",
    ] {
        field(frame, "errorFrame", &format!("properties/{key}"), Some)?;
    }

    let code = &frame["properties"]["code"];
    let codes = match (&code["const"], &code["enum"]) {
        (Value::String(code), _) => vec![code.as_str()],
        (_, Value::Array(codes)) => codes
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()
            .ok_or("errorFrame.properties.code.enum has a code that isn't a string")?,
        _ => return Err("errorFrame.properties.code has no const or enum".into()),
    };
    codes
        .into_iter()
        .map(|code| new_code(code, "", "", Vec::new()))
        .collect()
}

/// `HANDSHAKE_SOURCE_ID_IN_USE` → `SourceIdInUse`, without `prefix`.
fn new_code(code: &str, prefix: &str, doc: &str, fields: Vec<Field>) -> Result<Code, String> {
    let variant = pascal(
        &code
            .strip_prefix(prefix)
            .unwrap_or(code)
            .to_ascii_lowercase(),
    );
    if variant.is_empty() || variant == "Other" {
        return Err(format!("{code} can't name a variant"));
    }

    Ok(Code {
        code: code.to_owned(),
        doc: doc.to_owned(),
        variant,
        fields,
    })
}

fn connection(connection: &Value) -> Result<Connection, String> {
    let number = |key| field(connection, "connection", key, Value::as_u64);
    let text = |key| field(connection, "connection", key, Value::as_str).map(str::to_owned);
    // Discovery reads this TXT key by name.
    field(
        connection,
        "connection",
        "mdnsTxt/properties/protocol",
        Some,
    )?;

    Ok(Connection {
        protocol_version: number("protocolVersion")? as u32,
        default_port: number("defaultPort")? as u16,
        port_probe_limit: number("portProbeLimit")? as u16,
        max_payload_bytes: number("socketIo/maxPayloadBytes")? as usize,
        namespace: text("namespace")?,
        mdns_service_type: text("mdnsServiceType")?,
        error_channel: text("errorChannel")?,
        socket_io_path: text("socketIo/path")?,
    })
}

/// Fails on any manifest key the SDK neither reads nor ignores on purpose, so a sync can't add something the SDK
/// silently skips. Schemas typify turns into types are left out: their types follow the manifest.
fn check_keys(manifest: &Value) -> Result<(), String> {
    known_keys(
        manifest,
        "manifest",
        &[
            "connection",
            "descriptor",
            "handshakeErrors",
            "errorFrame",
            "events",
            "commands",
        ],
    )?;
    let connection = &manifest["connection"];
    known_keys(
        connection,
        "connection",
        &[
            "protocolVersion",
            "defaultPort",
            "portProbeLimit",
            "mdnsServiceType",
            "mdnsTxt",
            "socketIo",
            "namespace",
            "errorChannel",
            // Ignored: an HTTP header Node clients may send the descriptor in instead of the auth payload.
            "authHeader",
        ],
    )?;
    known_keys(
        &connection["socketIo"],
        "connection.socketIo",
        &[
            "path",
            "maxPayloadBytes",
            // Ignored: the client takes these from the Engine.IO open packet.
            "pingIntervalMs",
            "pingTimeoutMs",
        ],
    )?;
    known_keys(
        &connection["mdnsTxt"]["properties"],
        "connection.mdnsTxt.properties",
        &[
            "protocol", // Ignored: informational.
            "version",
        ],
    )?;

    for error in manifest["handshakeErrors"]["oneOf"]
        .as_array()
        .ok_or("handshakeErrors.oneOf is missing")?
    {
        known_keys(
            &error["properties"],
            "handshakeErrors.oneOf.properties",
            &["code", "metadata"],
        )?;
    }
    let frame = &manifest["errorFrame"]["properties"];
    known_keys(
        frame,
        "errorFrame.properties",
        &["event", "code", "message", "issues"],
    )?;
    known_keys(
        &frame["issues"]["items"]["properties"],
        "errorFrame.properties.issues.items.properties",
        &["code", "message", "path"],
    )?;

    for (name, entry) in object(&manifest["events"], "events")? {
        known_keys(
            entry,
            &format!("events.{name}"),
            &[
                "origin",
                "description",
                "deprecated",
                "maxRateHz",
                "payload",
            ],
        )?;
    }
    for (name, entry) in object(&manifest["commands"], "commands")? {
        known_keys(
            entry,
            &format!("commands.{name}"),
            &[
                "origin",
                "description",
                "deprecated",
                "timeoutMs",
                "input",
                "output",
                "errors",
                // Ignored: the reply envelope, which the SDK writes and the tests check replies against.
                "result",
            ],
        )?;
    }

    Ok(())
}

fn object<'a>(value: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or(format!("{at} is missing"))
}

fn known_keys(value: &Value, at: &str, known: &[&str]) -> Result<(), String> {
    match object(value, at)?
        .keys()
        .find(|key| !known.contains(&key.as_str()))
    {
        Some(key) => Err(format!(
            "the manifest has {at}.{key}, which the SDK doesn't handle yet; read it in the macro, or add it to the \
             keys ignored on purpose in check_keys"
        )),
        None => Ok(()),
    }
}

/// `key`, a `/`-separated path below `value`, read as `read` wants it. `at` names `value` in the error.
fn field<'a, T>(
    value: &'a Value,
    at: &str,
    key: &str,
    read: impl FnOnce(&'a Value) -> Option<T>,
) -> Result<T, String> {
    value
        .pointer(&format!("/{key}"))
        .and_then(read)
        .ok_or_else(|| format!("{at}.{} is missing", key.replace('/', ".")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_manifest_keys_the_sdk_does_not_handle() {
        let text = include_str!("../../../protocol/source-schema.json");
        let manifest: Value = serde_json::from_str(text).unwrap();
        assert_eq!(check_keys(&manifest), Ok(()));

        for pointer in [
            "",
            "/connection",
            "/events/telemetry.ownship",
            "/errorFrame/properties",
        ] {
            let mut changed = manifest.clone();
            changed.pointer_mut(pointer).unwrap()["unknown"] = json!(1);
            let error = check_keys(&changed).unwrap_err();
            assert!(error.contains(".unknown,"), "{pointer}: {error}");
        }
    }
}
