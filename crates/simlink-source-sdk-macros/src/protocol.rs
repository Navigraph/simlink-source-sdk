//! `protocol!("path/to/source-schema.json")`: Simlink's source manifest as exported Rust items.
//!
//! Emits the protocol's types (via typify), the `Event` enum, typed `Source` methods per event
//! and command, a handler trait and reply object per command, and the constants and event table
//! the SDK connects with. Every API item is a UniFFI item, so bindings never read the manifest.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use serde_json::{Map, Value};
use syn::{Error, LitStr};
use typify::{TypeSpace, TypeSpaceSettings};

pub fn expand(path: &LitStr) -> syn::Result<TokenStream> {
    let fail = |message: String| Error::new(path.span(), message);
    let dir = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| fail(e.to_string()))?;
    let file = std::path::Path::new(&dir).join(path.value());
    let text = std::fs::read_to_string(&file)
        .map_err(|e| fail(format!("reading {}: {e}", file.display())))?;
    let mut manifest: Value = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
    simplify(&mut manifest);

    let mut protocol = Protocol::default();
    protocol.load(&mut manifest).map_err(fail)?;
    let file = file.display().to_string();

    let (types, idents) = protocol.types().map_err(fail)?;
    let api = protocol.api(&idents);
    let tables = protocol.tables(&manifest["connection"]).map_err(fail)?;

    Ok(quote! {
        // Rebuild when the manifest changes.
        const _: &[u8] = include_bytes!(#file);

        #types
        #api
        #tables
    })
}

/// Strips what typify would turn into validating newtypes or required defaults; Simlink validates
/// and fills these in.
fn simplify(schema: &mut Value) {
    match schema {
        Value::Object(map) => {
            if let Some(Value::Object(properties)) = map.get_mut("properties") {
                properties.retain(|_, property| {
                    !(property.get("const").is_some() && property.get("default").is_some())
                });
                let kept: Vec<String> = properties.keys().cloned().collect();
                if let Some(Value::Array(required)) = map.get_mut("required") {
                    required.retain(|name| {
                        name.as_str()
                            .is_some_and(|name| kept.iter().any(|kept| kept == name))
                    });
                }
            }
            for key in [
                "minLength",
                "maxLength",
                "pattern",
                "format",
                "default",
                "$schema",
            ] {
                map.remove(key);
            }
            for (key, value) in map.iter_mut() {
                match (key.as_str(), value) {
                    // Keys here are property and definition names, never keywords to strip.
                    ("properties" | "$defs" | "patternProperties", Value::Object(schemas)) => {
                        schemas.values_mut().for_each(simplify)
                    }
                    (_, value) => simplify(value),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(simplify),
        _ => {}
    }
}

struct Root {
    name: Option<String>,
    schema: Value,
}

struct Channel {
    name: String,
    doc: String,
    deprecated: Option<String>,
}

struct EventChannel {
    channel: Channel,
    max_rate_hz: Option<f64>,
    payload: usize,
}

struct CommandChannel {
    channel: Channel,
    timeout_ms: u64,
    input: usize,
    output: usize,
    error: usize,
}

#[derive(Default)]
struct Protocol {
    defs: Map<String, Value>,
    roots: Vec<Root>,
    events: Vec<EventChannel>,
    commands: Vec<CommandChannel>,
}

impl Protocol {
    fn root(&mut self, name: Option<String>, schema: &mut Value) -> Result<usize, String> {
        if let Some(Value::Object(defs)) = schema
            .as_object_mut()
            .and_then(|schema| schema.remove("$defs"))
        {
            for (def, definition) in defs {
                // All definitions share one namespace of Rust types.
                match self.defs.get(&def) {
                    Some(existing) if *existing != definition => {
                        return Err(format!(
                            "$defs.{def} is defined differently by two channels"
                        ));
                    }
                    _ => self.defs.insert(def, definition),
                };
            }
        }
        self.roots.push(Root {
            name,
            schema: schema.clone(),
        });
        Ok(self.roots.len() - 1)
    }

    fn load(&mut self, manifest: &mut Value) -> Result<(), String> {
        self.load_descriptor(&mut manifest["descriptor"])?;

        // The rejection listener reads these frame fields by name.
        for key in [
            "event",
            "code",
            "message",
            "issues/items/properties/path",
            "issues/items/properties/message",
        ] {
            field(
                &manifest["errorFrame"],
                "errorFrame",
                &format!("properties/{key}"),
                Some,
            )?;
        }

        let events = manifest["events"]
            .as_object_mut()
            .ok_or("events is missing")?;
        for (name, entry) in events.iter_mut() {
            self.load_event(name, entry)?;
        }

        let commands = manifest["commands"]
            .as_object_mut()
            .ok_or("commands is missing")?;
        for (name, entry) in commands.iter_mut() {
            self.load_command(name, entry)?;
        }

        Ok(())
    }

    fn load_descriptor(&mut self, descriptor: &mut Value) -> Result<(), String> {
        let properties = descriptor["properties"]
            .as_object_mut()
            .ok_or("descriptor.properties is missing")?;
        properties
            .remove("capabilities")
            .ok_or("the SDK declares descriptor.capabilities, but the protocol has none")?;
        if let Some(required) = descriptor["required"].as_array_mut() {
            required.retain(|name| name != "capabilities");
        }
        descriptor["description"] =
            "Source identity presented to Simlink. The SDK declares the capabilities.".into();
        self.root(Some("SourceOptions".into()), descriptor)?;

        Ok(())
    }

    fn load_event(&mut self, name: &str, entry: &mut Value) -> Result<(), String> {
        let channel = channel(name, entry)?;
        let payload = self.root(
            name_if_object(&entry["payload"], &pascal(name)),
            &mut entry["payload"],
        )?;
        self.events.push(EventChannel {
            channel,
            max_rate_hz: entry["maxRateHz"].as_f64(),
            payload,
        });

        Ok(())
    }

    fn load_command(&mut self, name: &str, entry: &mut Value) -> Result<(), String> {
        let channel = channel(name, entry)?;
        let timeout_ms = field(entry, name, "timeoutMs", Value::as_u64)?;
        let base = pascal(name);
        let input = self.root(Some(format!("{base}Input")), &mut entry["input"])?;
        let output = self.root(
            name_if_object(&entry["output"], &format!("{base}Output")),
            &mut entry["output"],
        )?;

        // A single refusal code is a `const`; as a one-value enum it keeps its own type.
        let errors = &mut entry["errors"];
        if let Some(value) = errors
            .as_object_mut()
            .and_then(|errors| errors.remove("const"))
        {
            errors["enum"] = Value::Array(vec![value]);
        }
        let error = self.root(Some(format!("{base}Error")), errors)?;

        self.commands.push(CommandChannel {
            channel,
            timeout_ms,
            input,
            output,
            error,
        });

        Ok(())
    }

    /// The generated types, and the Rust type of each root by index.
    fn types(&self) -> Result<(TokenStream, Vec<TokenStream>), String> {
        let mut settings = TypeSpaceSettings::default();
        settings.with_struct_builder(false);
        let mut space = TypeSpace::new(&settings);

        let defs = self
            .defs
            .iter()
            .map(|(name, schema)| {
                Ok((
                    name.clone(),
                    serde_json::from_value(schema.clone()).map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        space.add_ref_types(defs).map_err(|e| e.to_string())?;

        let mut idents = Vec::new();
        for root in &self.roots {
            let schema: schemars::schema::Schema =
                serde_json::from_value(root.schema.clone()).map_err(|e| e.to_string())?;
            let id = match &root.name {
                Some(name) => space.add_type_with_name(&schema, Some(name.clone())),
                None => space.add_type(&schema),
            }
            .map_err(|e| e.to_string())?;
            idents.push(space.get_type(&id).map_err(|e| e.to_string())?.ident());
        }

        let mut file: syn::File = syn::parse2(space.to_stream()).map_err(|e| e.to_string())?;
        for item in &mut file.items {
            match item {
                syn::Item::Struct(item) => item
                    .attrs
                    .push(syn::parse_quote!(#[derive(::uniffi::Record)])),
                syn::Item::Enum(item) => item
                    .attrs
                    .push(syn::parse_quote!(#[derive(::uniffi::Enum)])),
                _ => {}
            }
        }

        Ok((quote!(#file), idents))
    }

    fn api(&self, idents: &[TokenStream]) -> TokenStream {
        let (variants, emits): (Vec<_>, Vec<_>) = self
            .events
            .iter()
            .map(|event| {
                let Channel { name, doc, deprecated } = &event.channel;
                let variant = format_ident!("{}", pascal(name));
                let method = format_ident!("emit_{}", snake(name));
                let payload = &idents[event.payload];
                let rate = event
                    .max_rate_hz
                    .map(|rate| format!("Sent at most {rate} Hz; the latest payload wins."))
                    .unwrap_or_default();
                let method_doc = sentences(&[doc, &rate]);
                let deprecated = self::deprecated(deprecated);
                (
                    quote!(#[doc = #doc] #deprecated #variant),
                    quote! {
                        #[doc = #method_doc]
                        #deprecated
                        pub fn #method(&self, payload: #payload) -> ::std::result::Result<(), crate::Error> {
                            self.emit_json(Event::#variant, &payload)
                        }
                    },
                )
            })
            .unzip();

        let commands = self.commands.iter().map(|command| {
            let Channel {
                name,
                doc,
                deprecated,
            } = &command.channel;
            let base = pascal(name);
            let (handler, reply) = (format_ident!("{base}Handler"), format_ident!("{base}Reply"));
            let method = format_ident!("on_{}", snake(name));
            let input = &idents[command.input];
            let output = &idents[command.output];
            let error = &idents[command.error];
            let timeout = format!(
                "Answer within {} ms. Register before `start`.",
                command.timeout_ms
            );
            let method_doc = sentences(&[doc, &timeout]);
            let handler_doc =
                format!("Answers `{name}` through `reply`, now or later from any thread.");
            let reply_doc = format!(
                "Answers one `{name}` command. Dropping it unanswered lets the command time out."
            );
            let deprecated = self::deprecated(deprecated);
            quote! {
                #[doc = #handler_doc]
                #[::uniffi::export(with_foreign)]
                pub trait #handler: Send + Sync {
                    /// Called on the SDK's thread, so keep it short.
                    fn handle(&self, input: #input, reply: ::std::sync::Arc<#reply>);
                }

                #[doc = #reply_doc]
                #[derive(::uniffi::Object)]
                pub struct #reply(crate::PendingReply);

                #[::uniffi::export]
                impl #reply {
                    /// Answers with the command's output.
                    pub fn ok(&self, value: #output) -> ::std::result::Result<(), crate::Error> {
                        self.0.ok(&value)
                    }

                    /// Refuses with one of the command's declared error codes.
                    pub fn refuse(&self, error: #error) -> ::std::result::Result<(), crate::Error> {
                        self.0.refuse(&error)
                    }
                }

                #[::uniffi::export]
                impl Source {
                    #[doc = #method_doc]
                    #deprecated
                    pub fn #method(
                        &self,
                        handler: ::std::sync::Arc<dyn #handler>,
                    ) -> ::std::result::Result<(), crate::Error> {
                        self.handle(#name, move |input, reply| {
                            handler.handle(input, ::std::sync::Arc::new(#reply(reply)))
                        })
                    }
                }
            }
        });

        quote! {
            use crate::Source;

            /// Event channels a source can declare.
            #[derive(Clone, Copy, Debug, PartialEq, Eq, ::uniffi::Enum)]
            pub enum Event {
                #( #variants, )*
            }

            impl Event {
                pub(crate) fn info(self) -> &'static crate::EventInfo {
                    // `EVENTS` lists the variants in order.
                    &EVENTS[self as usize]
                }
            }

            // The SDK keeps serving deprecated channels.
            #[allow(deprecated)]
            #[::uniffi::export]
            impl Source {
                #(#emits)*
            }

            #(#commands)*
        }
    }

    fn tables(&self, connection: &Value) -> Result<TokenStream, String> {
        let number = |key| field(connection, "connection", key, Value::as_u64);
        let text = |key| field(connection, "connection", key, Value::as_str);

        let protocol_version = number("protocolVersion")? as u32;
        let default_port = number("defaultPort")? as u16;
        let port_probe_limit = number("portProbeLimit")? as u16;
        let max_payload_bytes = number("socketIo/maxPayloadBytes")? as usize;
        let namespace = text("namespace")?;
        let mdns_service_type = text("mdnsServiceType")?;
        let error_channel = text("errorChannel")?;
        let socket_io_path = text("socketIo/path")?;
        // Discovery reads this TXT key by name.
        field(
            connection,
            "connection",
            "mdnsTxt/properties/protocol",
            Some,
        )?;

        let events = self.events.iter().map(|event| {
            let name = &event.channel.name;
            let rate = match event.max_rate_hz {
                Some(rate) => quote!(Some(#rate)),
                None => quote!(None),
            };
            quote!(crate::EventInfo { name: #name, max_rate_hz: #rate })
        });

        Ok(quote! {
            /// Wire protocol major. Simlink serves `/v{PROTOCOL_VERSION}/source` and advertises it in mDNS TXT.
            pub const PROTOCOL_VERSION: u32 = #protocol_version;
            pub(crate) const NAMESPACE: &str = #namespace;
            pub(crate) const DEFAULT_PORT: u16 = #default_port;
            pub(crate) const PORT_PROBE_LIMIT: u16 = #port_probe_limit;
            pub(crate) const MDNS_SERVICE_TYPE: &str = #mdns_service_type;
            pub(crate) const ERROR_CHANNEL: &str = #error_channel;
            pub(crate) const SOCKET_IO_PATH: &str = #socket_io_path;
            /// Simlink drops the connection on a larger message.
            pub(crate) const MAX_PAYLOAD_BYTES: usize = #max_payload_bytes;
            pub(crate) const EVENTS: &[crate::EventInfo] = &[#(#events),*];
        })
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

fn deprecated(reason: &Option<String>) -> Option<TokenStream> {
    match reason.as_deref()? {
        "" => Some(quote!(#[deprecated])),
        reason => Some(quote!(#[deprecated = #reason])),
    }
}

/// Joins the non-empty parts as sentences, so a channel without a description still reads well.
fn sentences(parts: &[&str]) -> String {
    parts
        .iter()
        .map(|part| part.trim().trim_end_matches('.'))
        .filter(|part| !part.is_empty())
        .map(|part| format!("{part}."))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Objects get a channel-derived name; arrays and maps stay `Vec<T>` and `HashMap<String, T>`.
fn name_if_object(schema: &Value, name: &str) -> Option<String> {
    schema.get("properties").map(|_| name.to_owned())
}

/// `telemetry.ownship` → `telemetry`, `ownship`.
fn words(name: &str) -> impl Iterator<Item = &str> {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
}

/// `telemetry.ownship` → `TelemetryOwnship`.
fn pascal(name: &str) -> String {
    words(name)
        .map(|word| word[..1].to_ascii_uppercase() + &word[1..])
        .collect()
}

/// `telemetry.ownship` → `telemetry_ownship`.
fn snake(name: &str) -> String {
    words(name).collect::<Vec<_>>().join("_")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn simplify_strips_keywords_but_keeps_properties_named_like_them() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "format": { "type": "string", "format": "date-time" },
                "pattern": { "type": "string", "pattern": "^a$", "maxLength": 4 },
                "kind": { "type": "string", "const": "standard", "default": "standard" },
            },
            "required": ["format", "kind"],
            "$defs": { "default": { "type": "string", "minLength": 1 } },
        });
        simplify(&mut schema);

        assert_eq!(
            schema,
            json!({
                "type": "object",
                "properties": { "format": { "type": "string" }, "pattern": { "type": "string" } },
                "required": ["format"],
                "$defs": { "default": { "type": "string" } },
            })
        );
    }
}
