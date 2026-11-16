//! Reads the manifest into the model the generators work from, failing on anything the SDK doesn't handle.

use std::collections::BTreeMap;

use heck::ToUpperCamelCase;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

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
#[derive(Clone)]
pub struct Field {
    pub key: String,
    pub ty: FieldType,
    pub required: bool,
}

/// The JSON types every binding has.
#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FieldType {
    String,
    Integer,
    Number,
    Boolean,
    Array { items: Box<FieldType> },
}

/// What the SDK connects with.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub protocol_version: u32,
    pub default_port: u16,
    pub port_probe_limit: u16,
    pub mdns_service_type: String,
    pub socket_io: SocketIo,
    pub namespace: String,
    pub error_channel: String,
    #[serde(rename = "mdnsTxt")]
    _mdns_txt: Object<MdnsTxt>,
    /// An HTTP header Node clients may send the descriptor in instead of the auth payload.
    #[serde(rename = "authHeader")]
    _auth_header: Ignored,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocketIo {
    pub path: String,
    pub max_payload_bytes: usize,
    /// The client takes these from the Engine.IO open packet.
    #[serde(rename = "pingIntervalMs")]
    _ping_interval_ms: Ignored,
    #[serde(rename = "pingTimeoutMs")]
    _ping_timeout_ms: Ignored,
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
    let root: Root = strict("", &mut serde_json::Deserializer::from_str(text))?;
    let Protocol {
        descriptor,
        handshake_errors,
        error_frame,
        events,
        commands,
    } = root.properties;

    let mut handshake_codes = Vec::new();
    let handshake_errors: OneOf<Object<HandshakeError>> = handshake_errors.read(&root.defs)?;
    for error in handshake_errors.one_of {
        handshake_codes.extend(error.properties.codes()?);
    }
    let error_frame: Object<ErrorFrame> = error_frame.read(&root.defs)?;

    Ok(Manifest {
        descriptor: source_options(descriptor.resolve(&root.defs)?)?,
        events: events.properties.into_iter().map(Event::from).collect(),
        commands: commands.properties.into_iter().map(Command::from).collect(),
        handshake_errors: handshake_codes,
        rejection_codes: error_frame.properties.code.codes("", Vec::new())?,
        connection: root.connection,
        defs: root.defs,
    })
}

#[derive(Deserialize)]
struct Root {
    #[serde(rename = "x-connection")]
    connection: Connection,
    properties: Protocol,
    #[serde(rename = "$defs")]
    defs: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Protocol {
    descriptor: Ref,
    handshake_errors: Ref,
    error_frame: Ref,
    events: Object<BTreeMap<String, EventEntry>>,
    commands: Object<BTreeMap<String, CommandEntry>>,
}

#[derive(Deserialize)]
struct Ref {
    #[serde(rename = "$ref")]
    path: String,
}

impl Ref {
    fn resolve<'a>(&self, defs: &'a Map<String, Value>) -> Result<&'a Value, String> {
        self.path
            .strip_prefix("#/$defs/")
            .and_then(|name| defs.get(name))
            .ok_or(format!("{} is missing", self.path))
    }

    /// The definition, read as `T`.
    fn read<T: DeserializeOwned>(&self, defs: &Map<String, Value>) -> Result<T, String> {
        let at = format!("{}.", self.path.trim_start_matches("#/").replace('/', "."));
        strict(&at, self.resolve(defs)?)
    }
}

/// An object schema, of which the SDK reads the `properties`.
#[derive(Deserialize)]
struct Object<T> {
    properties: T,
}

#[derive(Deserialize)]
struct OneOf<T> {
    #[serde(rename = "oneOf")]
    one_of: Vec<T>,
}

/// A key the SDK knows about and leaves alone, with the reason where it's declared.
type Ignored = Option<Value>;

#[allow(dead_code)]
#[derive(Deserialize)]
struct MdnsTxt {
    /// Discovery reads this key by name.
    protocol: Value,
    /// Informational.
    version: Ignored,
}

/// The SDK only emits and answers on behalf of a source; other channels would point the wrong way.
#[derive(Deserialize)]
enum Origin {
    #[serde(rename = "source")]
    Source,
}

#[derive(Deserialize)]
struct EventEntry {
    #[serde(rename = "x-origin")]
    _origin: Origin,
    #[serde(default)]
    description: String,
    #[serde(default)]
    deprecated: bool,
    #[serde(rename = "x-deprecation", default)]
    deprecation: String,
    #[serde(rename = "x-max-rate-hz")]
    max_rate_hz: Option<f64>,
    properties: EventParts,
}

#[derive(Deserialize)]
struct EventParts {
    payload: Value,
}

#[derive(Deserialize)]
struct CommandEntry {
    #[serde(rename = "x-origin")]
    _origin: Origin,
    #[serde(default)]
    description: String,
    #[serde(default)]
    deprecated: bool,
    #[serde(rename = "x-deprecation", default)]
    deprecation: String,
    #[serde(rename = "x-timeout-ms")]
    timeout_ms: u64,
    properties: CommandParts,
}

#[derive(Deserialize)]
struct CommandParts {
    input: Value,
    output: Value,
    errors: Value,
    /// The reply envelope, which the SDK writes and the tests check replies against.
    #[serde(rename = "result")]
    _result: Ignored,
}

impl Channel {
    fn new(name: String, doc: String, deprecated: bool, deprecation: String) -> Channel {
        Channel {
            name,
            doc,
            deprecated: deprecated.then_some(deprecation),
        }
    }
}

impl From<(String, EventEntry)> for Event {
    fn from((name, entry): (String, EventEntry)) -> Event {
        Event {
            channel: Channel::new(name, entry.description, entry.deprecated, entry.deprecation),
            max_rate_hz: entry.max_rate_hz,
            payload: entry.properties.payload,
        }
    }
}

impl From<(String, CommandEntry)> for Command {
    fn from((name, entry): (String, CommandEntry)) -> Command {
        Command {
            channel: Channel::new(name, entry.description, entry.deprecated, entry.deprecation),
            timeout_ms: entry.timeout_ms,
            input: entry.properties.input,
            output: entry.properties.output,
            refusal: entry.properties.errors,
        }
    }
}

#[derive(Deserialize)]
struct HandshakeError {
    code: CodeSchema,
    #[serde(default)]
    metadata: Metadata,
}

#[derive(Default, Deserialize)]
struct Metadata {
    properties: BTreeMap<String, FieldType>,
    #[serde(default)]
    required: Vec<String>,
}

impl HandshakeError {
    fn codes(self) -> Result<Vec<Code>, String> {
        let Metadata {
            properties,
            required,
        } = self.metadata;
        let fields = properties.into_iter().map(|(key, ty)| Field {
            required: required.contains(&key),
            key,
            ty,
        });
        self.code.codes("HANDSHAKE_", fields.collect())
    }
}

/// The frame fields the rejection listener reads by name, which must be present.
#[allow(dead_code)]
#[derive(Deserialize)]
struct ErrorFrame {
    event: Value,
    message: Value,
    code: CodeSchema,
    issues: Issues,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Issues {
    items: Object<Issue>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Issue {
    path: Value,
    code: Value,
    message: Value,
}

/// A string schema whose `const` or `enum` lists codes.
#[derive(Deserialize)]
struct CodeSchema {
    #[serde(rename = "const")]
    constant: Option<String>,
    #[serde(rename = "enum", default)]
    values: Vec<String>,
    #[serde(default)]
    description: String,
}

impl CodeSchema {
    /// Each code, named as a variant without `prefix`: `HANDSHAKE_SOURCE_ID_IN_USE` → `SourceIdInUse`.
    fn codes(self, prefix: &str, fields: Vec<Field>) -> Result<Vec<Code>, String> {
        let codes: Vec<_> = self.constant.into_iter().chain(self.values).collect();
        if codes.is_empty() {
            return Err("a code schema has no const or enum".into());
        }
        codes
            .into_iter()
            .map(|code| {
                let variant = code
                    .strip_prefix(prefix)
                    .unwrap_or(&code)
                    .to_upper_camel_case();
                if variant.is_empty() || variant == "Other" {
                    return Err(format!("{code} can't name a variant"));
                }
                Ok(Code {
                    doc: self.description.clone(),
                    variant,
                    fields: fields.clone(),
                    code,
                })
            })
            .collect()
    }
}

fn source_options(descriptor: &Value) -> Result<Value, String> {
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

/// Keywords that only make an object holding the protocol's parts a schema itself.
const WRAPPER: &[&str] = &[
    "$schema",
    "title",
    "type",
    "required",
    "additionalProperties",
];

/// Deserializes `T`, failing on any key it doesn't read, so a sync can't add something the SDK silently skips. `at`
/// prefixes paths in errors. Schemas typify turns into types are read whole: their types follow the manifest.
fn strict<'de, T: Deserialize<'de>>(
    at: &str,
    deserializer: impl Deserializer<'de>,
) -> Result<T, String> {
    let mut unknown = None;
    let mut ignored = |path: serde_ignored::Path| {
        let wrapper = matches!(&path, serde_ignored::Path::Map { key, .. } if WRAPPER.contains(&key.as_str()));
        if !wrapper && unknown.is_none() {
            unknown = Some(path.to_string());
        }
    };
    let value = serde_path_to_error::deserialize(serde_ignored::Deserializer::new(
        deserializer,
        &mut ignored,
    ))
    .map_err(|e| format!("{at}{}: {}", e.path(), e.inner()))?;
    match unknown {
        Some(path) => Err(format!(
            "the manifest has {at}{path}, which the SDK doesn't handle yet; read it in the macro, or declare it \
             `Ignored` with the reason"
        )),
        None => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_manifest_keys_the_sdk_does_not_handle() {
        let text = include_str!("../../../protocol/source-schema.json");
        assert!(read(text).is_ok());

        let manifest: Value = serde_json::from_str(text).unwrap();
        for pointer in [
            "",
            "/x-connection",
            "/x-connection/socketIo",
            "/properties",
            "/properties/events/properties/telemetry.ownship",
            "/properties/commands/properties/atis.get/properties",
            "/$defs/HandshakeError/oneOf/1/properties",
            "/$defs/SourceErrorFrame/properties",
        ] {
            let mut changed = manifest.clone();
            changed.pointer_mut(pointer).unwrap()["unknown"] = json!(1);
            let error = read(&changed.to_string()).err().unwrap();
            assert!(error.contains("unknown, which"), "{pointer}: {error}");
        }
    }
}
