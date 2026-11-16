//! Reads the manifest into the model the generators work from, failing on anything the SDK doesn't handle.

use serde_json::{Map, Value};

use heck::ToUpperCamelCase;

pub struct Channel {
    pub name: String,
    pub doc: String,
    pub deprecated: Option<String>,
}

pub struct Event {
    pub channel: Channel,
    pub max_rate_hz: Option<f64>,
    pub payload: Value,
}

pub struct Command {
    pub channel: Channel,
    pub timeout_ms: u64,
    pub input: Value,
    pub output: Value,
    pub refusal: Value,
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
    pub descriptor: Value,
    pub events: Vec<Event>,
    pub commands: Vec<Command>,
    pub handshake_errors: Vec<Code>,
    pub rejection_codes: Vec<Code>,
    pub connection: Connection,
    /// The types every schema above refers to by `#/$defs/<name>`.
    pub defs: Map<String, Value>,
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

    Ok(Manifest {
        descriptor: descriptor(part(&manifest, "descriptor")?)?,
        events: channels(&manifest, "events")?
            .iter()
            .map(|(name, entry)| event(name, entry))
            .collect::<Result<_, _>>()?,
        commands: channels(&manifest, "commands")?
            .iter()
            .map(|(name, entry)| command(name, entry))
            .collect::<Result<_, _>>()?,
        handshake_errors: handshake_errors(&manifest)?
            .iter()
            .map(handshake_error)
            .collect::<Result<_, _>>()?,
        rejection_codes: rejection_codes(part(&manifest, "errorFrame")?)?,
        connection: connection(&manifest["x-connection"])?,
        defs: object(&manifest["$defs"], "$defs")?.clone(),
    })
}

/// The schema of the protocol's `key`, following its `$ref`.
fn part<'a>(manifest: &'a Value, key: &str) -> Result<&'a Value, String> {
    let schema = &manifest["properties"][key];
    match schema["$ref"].as_str() {
        None => Ok(schema),
        Some(reference) => reference
            .strip_prefix('#')
            .and_then(|pointer| manifest.pointer(pointer))
            .ok_or(format!("{key} refers to {reference}, which is missing")),
    }
}

/// The `events` or `commands` by name.
fn channels<'a>(manifest: &'a Value, kind: &str) -> Result<&'a Map<String, Value>, String> {
    object(&part(manifest, kind)?["properties"], kind)
}

fn handshake_errors(manifest: &Value) -> Result<&Vec<Value>, String> {
    part(manifest, "handshakeErrors")?["oneOf"]
        .as_array()
        .ok_or("handshakeErrors.oneOf is missing".into())
}

fn descriptor(descriptor: &Value) -> Result<Value, String> {
    let mut schema = descriptor.clone();
    schema["properties"]
        .as_object_mut()
        .ok_or("descriptor.properties is missing")?
        .remove("capabilities")
        .ok_or("the SDK declares descriptor.capabilities, but the protocol has none")?;
    if let Some(required) = schema["required"].as_array_mut() {
        required.retain(|name| name != "capabilities");
    }
    schema["title"] = "SourceOptions".into();
    schema["description"] =
        "Source identity presented to Simlink. The SDK declares the capabilities.".into();

    Ok(schema)
}

fn event(name: &str, entry: &Value) -> Result<Event, String> {
    Ok(Event {
        channel: channel(name, entry)?,
        max_rate_hz: entry["x-max-rate-hz"].as_f64(),
        payload: field(entry, name, "properties/payload", Some)?.clone(),
    })
}

fn command(name: &str, entry: &Value) -> Result<Command, String> {
    let schema = |key: &str| field(entry, name, &format!("properties/{key}"), Some).cloned();
    Ok(Command {
        channel: channel(name, entry)?,
        timeout_ms: field(entry, name, "x-timeout-ms", Value::as_u64)?,
        input: schema("input")?,
        output: schema("output")?,
        refusal: schema("errors")?,
    })
}

/// What the generated API needs from a channel entry. The SDK only emits and answers on behalf of a source; other
/// channels would point the wrong way.
fn channel(name: &str, entry: &Value) -> Result<Channel, String> {
    match entry["x-origin"].as_str() {
        Some("source") => {}
        origin => {
            return Err(format!(
                "{name} has origin {origin:?}; the SDK only handles source channels"
            ));
        }
    }
    let reason = entry["x-deprecation"].as_str().unwrap_or_default();
    let deprecated = (entry["deprecated"] == true).then(|| reason.to_owned());

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
    let variant = code
        .strip_prefix(prefix)
        .unwrap_or(code)
        .to_upper_camel_case();
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

/// Keywords that only make an object containing schemas a schema itself.
const WRAPPER: &[&str] = &[
    "$schema",
    "title",
    "type",
    "required",
    "additionalProperties",
];

/// Fails on any manifest key the SDK neither reads nor ignores on purpose, so a sync can't add something the SDK
/// silently skips. Schemas typify turns into types are left out: their types follow the manifest.
fn check_keys(manifest: &Value) -> Result<(), String> {
    known_keys(
        manifest,
        "manifest",
        &[WRAPPER, &["properties", "$defs", "x-connection"]].concat(),
    )?;
    known_keys(
        &manifest["properties"],
        "manifest.properties",
        &[
            "descriptor",
            "handshakeErrors",
            "errorFrame",
            "events",
            "commands",
        ],
    )?;
    let connection = &manifest["x-connection"];
    known_keys(
        connection,
        "x-connection",
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
        "x-connection.socketIo",
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
        "x-connection.mdnsTxt.properties",
        // Ignored: `version`, which is informational.
        &["protocol", "version"],
    )?;

    for error in handshake_errors(manifest)? {
        known_keys(
            &error["properties"],
            "handshakeErrors.oneOf.properties",
            &["code", "metadata"],
        )?;
    }
    let frame = &part(manifest, "errorFrame")?["properties"];
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

    let channel_keys = [
        WRAPPER,
        &[
            "properties",
            "description",
            "deprecated",
            "x-deprecation",
            "x-origin",
        ],
    ]
    .concat();
    for (name, entry) in channels(manifest, "events")? {
        let at = format!("events.{name}");
        known_keys(
            entry,
            &at,
            &[&channel_keys[..], &["x-max-rate-hz"]].concat(),
        )?;
        known_keys(
            &entry["properties"],
            &format!("{at}.properties"),
            &["payload"],
        )?;
    }
    for (name, entry) in channels(manifest, "commands")? {
        let at = format!("commands.{name}");
        known_keys(entry, &at, &[&channel_keys[..], &["x-timeout-ms"]].concat())?;
        known_keys(
            &entry["properties"],
            &format!("{at}.properties"),
            &[
                "input", "output", "errors",
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
            "/properties",
            "/x-connection",
            "/properties/events/properties/telemetry.ownship",
            "/properties/events/properties/telemetry.ownship/properties",
            "/$defs/SourceErrorFrame/properties",
        ] {
            let mut changed = manifest.clone();
            changed.pointer_mut(pointer).unwrap()["unknown"] = json!(1);
            let error = check_keys(&changed).unwrap_err();
            assert!(error.contains(".unknown,"), "{pointer}: {error}");
        }
    }
}
