//! Reads the manifest into the model the generators work from, failing on anything the SDK doesn't handle.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

#[derive(Deserialize)]
pub struct Event {
    #[serde(rename = "x-origin")]
    _origin: Origin,
    #[serde(default)]
    pub description: String,
    /// `x-deprecation` says the same, with the reason.
    #[serde(rename = "deprecated")]
    _deprecated: Ignored,
    #[serde(rename = "x-deprecation")]
    pub deprecation: Option<String>,
    #[serde(rename = "x-max-rate-hz")]
    pub max_rate_hz: Option<f64>,
    pub properties: EventParts,
}

#[derive(Deserialize)]
pub struct EventParts {
    pub payload: Value,
}

#[derive(Deserialize)]
pub struct Command {
    #[serde(rename = "x-origin")]
    _origin: Origin,
    #[serde(default)]
    pub description: String,
    /// `x-deprecation` says the same, with the reason.
    #[serde(rename = "deprecated")]
    _deprecated: Ignored,
    #[serde(rename = "x-deprecation")]
    pub deprecation: Option<String>,
    #[serde(rename = "x-timeout-ms")]
    pub timeout_ms: u64,
    pub properties: CommandParts,
}

#[derive(Deserialize)]
pub struct CommandParts {
    pub input: Value,
    pub output: Value,
    pub errors: Value,
    /// The reply envelope, which the SDK writes and the tests check replies against.
    #[serde(rename = "result")]
    _result: Ignored,
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
    /// The descriptor's identity part, which integrators fill in.
    pub options: Value,
    pub handshake_errors: Value,
    /// The descriptor's capabilities, which the SDK declares.
    pub capabilities: Value,
    pub error_frame: Value,
    /// By channel name.
    pub events: BTreeMap<String, Event>,
    pub commands: BTreeMap<String, Command>,
    pub connection: Connection,
    /// The types every schema above refers to by `#/$defs/<name>`.
    pub defs: Map<String, Value>,
}

impl Manifest {
    /// The name and description of every event, then every command.
    pub fn channels(&self) -> impl Iterator<Item = (&String, &String)> {
        let events = self
            .events
            .iter()
            .map(|(name, event)| (name, &event.description));
        events.chain(
            self.commands
                .iter()
                .map(|(name, command)| (name, &command.description)),
        )
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

    let Descriptor {
        all_of: (options, capabilities),
        ..
    } = descriptor.read(&root.defs)?;

    Ok(Manifest {
        options,
        handshake_errors,
        capabilities: capabilities.properties.capabilities,
        error_frame,
        events: events.properties,
        commands: commands.properties,
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
    handshake_errors: Value,
    error_frame: Value,
    events: Object<BTreeMap<String, Event>>,
    commands: Object<BTreeMap<String, Command>>,
}

#[derive(Deserialize)]
struct Ref {
    #[serde(rename = "$ref")]
    path: String,
}

impl Ref {
    /// The definition, read as `T`.
    fn read<T: DeserializeOwned>(&self, defs: &Map<String, Value>) -> Result<T, String> {
        let definition = self
            .path
            .strip_prefix("#/$defs/")
            .and_then(|name| defs.get(name))
            .ok_or(format!("{} is missing", self.path))?;
        let at = format!("{}.", self.path.trim_start_matches("#/").replace('/', "."));
        strict(&at, definition)
    }
}

/// An object schema, of which the SDK reads the `properties`.
#[derive(Deserialize)]
struct Object<T> {
    properties: T,
}

/// A key the SDK knows about and leaves alone, with the reason where it's declared.
type Ignored = Option<Value>;

/// The options integrators fill in, and the capabilities the SDK declares.
#[derive(Deserialize)]
struct Descriptor {
    #[serde(rename = "allOf")]
    all_of: (Value, Object<Capabilities>),
    /// The SDK documents the options and capabilities instead.
    #[serde(rename = "description")]
    _description: Ignored,
}

#[derive(Deserialize)]
struct Capabilities {
    capabilities: Value,
}

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
        let text = include_str!("../../simlink-source-sdk/protocol/source-schema.json");
        assert!(read(text).is_ok());

        let manifest: Value = serde_json::from_str(text).unwrap();
        for pointer in [
            "",
            "/x-connection",
            "/x-connection/socketIo",
            "/properties",
            "/properties/events/properties/telemetry.ownship",
            "/properties/commands/properties/atis.get/properties",
            "/$defs/SourceDescriptor",
            "/$defs/SourceDescriptor/allOf/1/properties",
        ] {
            let mut changed = manifest.clone();
            changed.pointer_mut(pointer).unwrap()["unknown"] = json!(1);
            let error = read(&changed.to_string()).err().unwrap();
            assert!(error.contains("unknown, which"), "{pointer}: {error}");
        }
    }
}
